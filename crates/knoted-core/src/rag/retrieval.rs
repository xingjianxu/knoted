//! Version-checked lexical and dense retrieval over the same immutable chunks.

use std::collections::HashMap;

use serde::Deserialize;
use serde_json::json;
use sqlx::{Postgres, QueryBuilder, types::Json};
use uuid::Uuid;

use crate::{
    context::AppContext,
    search::{
        chunks::chunk_index_uid,
        meili::{self, MeiliClient, SourceType},
    },
};

use super::{
    chunking::{CHUNKER_REVISION, ChunkLocation},
    embedding::{MODEL, MODEL_REVISION},
    vector::{self, SearchVectors},
};

const CANDIDATES: usize = 40;
const MAX_EVIDENCE: usize = 12;
const MAX_PER_SOURCE: usize = 4;
const RRF_K: f64 = 60.0;
const LEXICAL_PAGE: usize = 100;
const LEXICAL_WINDOW: usize = 1000;

#[derive(Debug, thiserror::Error)]
pub enum DomainError {
    #[error("invalid retrieval query")]
    Invalid,
    #[error("retrieval engines unavailable")]
    Unavailable,
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Lexical(#[from] meili::DomainError),
    #[error(transparent)]
    Vector(#[from] vector::DomainError),
}

#[derive(Debug, Default, Clone)]
pub struct Scope {
    /// Empty selects all evidence types. Projects and chat are outside RAG.
    pub source_types: Vec<SourceType>,
    pub project_ids: Vec<Uuid>,
    pub tag_ids: Vec<Uuid>,
}

#[derive(Debug)]
pub struct RetrievalQuery {
    pub text: String,
    pub embedding: Option<Vec<f32>>,
    pub scope: Scope,
}

#[derive(Debug)]
pub struct EvidenceSet {
    pub items: Vec<Evidence>,
}

#[derive(Debug)]
pub struct Evidence {
    pub chunk_id: Uuid,
    pub chunk_set_id: Uuid,
    pub source_id: Uuid,
    pub source_type: SourceType,
    pub source_version: i64,
    pub title: String,
    pub text: String,
    pub locations: Vec<ChunkLocation>,
    pub score: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
struct Candidate {
    chunk_id: Uuid,
    chunk_set_id: Uuid,
    source_version: i64,
}

#[derive(Clone, sqlx::FromRow)]
struct Matched {
    chunk_id: Uuid,
    chunk_set_id: Uuid,
    source_version: i64,
    source_id: Uuid,
    source_type: String,
    title: String,
    text: String,
    locations: Json<Vec<ChunkLocation>>,
    lexical_published: bool,
}

impl Matched {
    fn identity(&self) -> Candidate {
        Candidate {
            chunk_id: self.chunk_id,
            chunk_set_id: self.chunk_set_id,
            source_version: self.source_version,
        }
    }
}

pub async fn retrieve(ctx: &AppContext, query: RetrievalQuery) -> Result<EvidenceSet, DomainError> {
    let text = query.text.trim();
    if text.is_empty() || text.chars().count() > 20_000 {
        return Err(DomainError::Invalid);
    }
    let mut scope = query.scope;
    let permitted = [SourceType::File, SourceType::Note, SourceType::Task];
    if scope.source_types.is_empty() {
        scope.source_types = permitted.to_vec();
    } else {
        scope.source_types.retain(|kind| permitted.contains(kind));
    }
    if scope.source_types.is_empty() {
        return Ok(EvidenceSet { items: Vec::new() });
    }

    let (lexical, dense) = tokio::join!(lexical(ctx, text, &scope), async {
        let Some(embedding) = query.embedding else {
            return Ok(None);
        };
        let matches = vector::search_vectors(
            ctx,
            SearchVectors {
                model: MODEL.into(),
                model_revision: MODEL_REVISION.into(),
                query: embedding,
                source_types: scope.source_types.clone(),
                project_ids: scope.project_ids.clone(),
                tag_ids: scope.tag_ids.clone(),
                limit: CANDIDATES as u32,
            },
        )
        .await?;
        Ok::<_, DomainError>(Some(
            matches
                .into_iter()
                .map(|hit| Candidate {
                    chunk_id: hit.chunk_id,
                    chunk_set_id: hit.chunk_set_id,
                    source_version: hit.source_version,
                })
                .collect::<Vec<_>>(),
        ))
    });
    let (lexical, dense) = match (lexical, dense) {
        (Ok(lexical), Ok(dense)) => (lexical, dense.unwrap_or_default()),
        (Ok(lexical), Err(_)) => (lexical, Vec::new()),
        (Err(_), Ok(Some(dense))) => (Vec::new(), dense),
        (Err(error), _) => return Err(error),
    };

    // Recheck after both engines answered. Read evidence from PostgreSQL
    // rather than from engine text, maintaining the same snapshot and scope.
    let combined: Vec<_> = lexical.iter().chain(&dense).copied().collect();
    let valid_dense = visible(ctx, &scope, &combined).await?;
    let valid_lexical = valid_dense
        .iter()
        .filter(|(_, row)| row.lexical_published)
        .map(|(id, row)| (*id, row.clone()))
        .collect();
    Ok(EvidenceSet {
        items: fuse(&lexical, &dense, valid_lexical, valid_dense),
    })
}

async fn lexical(
    ctx: &AppContext,
    text: &str,
    scope: &Scope,
) -> Result<Vec<Candidate>, DomainError> {
    let client = MeiliClient::from_context(ctx)?;
    let path = format!("indexes/{}/search", chunk_index_uid(ctx));
    let types: Vec<_> = scope
        .source_types
        .iter()
        .map(|kind| kind.as_str())
        .collect();
    let filter = format!("source_type IN [{}]", types.join(","));
    let mut candidates = Vec::new();
    for offset in (0..LEXICAL_WINDOW).step_by(LEXICAL_PAGE) {
        let result = client
            .send(client.request(reqwest::Method::POST, &path).json(&json!({
                "q": text,
                "offset": offset,
                "limit": LEXICAL_PAGE,
                "filter": filter,
                "attributesToRetrieve": ["chunk_id", "chunk_set_id", "source_version"],
            })))
            .await?;
        let hits: Vec<Candidate> =
            serde_json::from_value(result["hits"].clone()).map_err(|_| DomainError::Unavailable)?;
        let done = hits.len() < LEXICAL_PAGE;
        let current = visible(ctx, scope, &hits).await?;
        for hit in hits {
            if current.get(&hit).is_some_and(|row| row.lexical_published)
                && !candidates.contains(&hit)
            {
                candidates.push(hit);
            }
            if candidates.len() == CANDIDATES {
                return Ok(candidates);
            }
        }
        if done {
            break;
        }
    }
    Ok(candidates)
}

async fn visible(
    ctx: &AppContext,
    scope: &Scope,
    hits: &[Candidate],
) -> Result<HashMap<Candidate, Matched>, sqlx::Error> {
    if hits.is_empty() {
        return Ok(HashMap::new());
    }
    let ids: Vec<_> = hits.iter().map(|hit| hit.chunk_id).collect();
    let sets: Vec<_> = hits.iter().map(|hit| hit.chunk_set_id).collect();
    let versions: Vec<_> = hits.iter().map(|hit| hit.source_version).collect();
    let types: Vec<_> = scope
        .source_types
        .iter()
        .map(|kind| kind.as_str())
        .collect();
    let mut sql: QueryBuilder<'_, Postgres> = QueryBuilder::new(
        "SELECT c.id AS chunk_id,
        s.id AS chunk_set_id,s.source_version,s.source_id,s.source_type,c.text,c.locations,
        COALESCE(n.title,t.title,regexp_replace(f.relative_path,'^.*/',''),'') AS title,
        EXISTS (SELECT 1 FROM rag_chunk_publications p
            WHERE p.chunk_set_id=s.id AND p.status='published' AND p.index_uid=",
    );
    sql.push_bind(chunk_index_uid(ctx))
        .push(") AS lexical_published FROM unnest(");
    sql.push_bind(ids)
        .push("::uuid[],")
        .push_bind(sets)
        .push("::uuid[],")
        .push_bind(versions)
        .push(
            "::bigint[]) AS h(chunk_id,set_id,source_version)
        JOIN rag_chunks c ON c.id=h.chunk_id AND c.chunk_set_id=h.set_id
        JOIN rag_chunk_sets s ON s.id=c.chunk_set_id AND s.source_version=h.source_version
        JOIN current_rag_sources live ON live.source_type=s.source_type
          AND live.source_id=s.source_id AND live.source_version=s.source_version
          AND live.extraction_id=s.extraction_id
        LEFT JOIN notes n ON s.source_type='note' AND n.id=s.source_id
        LEFT JOIN tasks t ON s.source_type='task' AND t.id=s.source_id
        LEFT JOIN file_sources f ON s.source_type='file' AND f.id=s.source_id
        WHERE s.chunker_revision=",
        )
        .push_bind(CHUNKER_REVISION)
        .push(" AND s.source_type=ANY(")
        .push_bind(types)
        .push(")");
    vector::push_scope(&mut sql, scope.project_ids.clone(), scope.tag_ids.clone());
    let rows: Vec<Matched> = sql.build_query_as().fetch_all(ctx.pool()).await?;
    Ok(rows.into_iter().map(|row| (row.identity(), row)).collect())
}

fn fuse(
    lexical: &[Candidate],
    dense: &[Candidate],
    mut valid_lexical: HashMap<Candidate, Matched>,
    mut valid_dense: HashMap<Candidate, Matched>,
) -> Vec<Evidence> {
    let mut scores: HashMap<Candidate, (f64, Matched)> = HashMap::new();
    for (branch, allowed) in [(lexical, &mut valid_lexical), (dense, &mut valid_dense)] {
        let mut rank = 0;
        for candidate in branch {
            if let Some(row) = allowed.remove(candidate) {
                rank += 1;
                scores.entry(*candidate).or_insert((0.0, row)).0 += 1.0 / (RRF_K + f64::from(rank));
            }
        }
    }
    let mut ranked: Vec<_> = scores.into_values().collect();
    ranked.sort_by(|(a_score, a), (b_score, b)| {
        b_score.total_cmp(a_score).then(a.chunk_id.cmp(&b.chunk_id))
    });
    let mut counts = HashMap::new();
    ranked
        .into_iter()
        .filter_map(|(score, row)| {
            let source_type = SourceType::parse(&row.source_type)?;
            let count = counts.entry((source_type, row.source_id)).or_insert(0);
            if *count >= MAX_PER_SOURCE {
                return None;
            }
            *count += 1;
            Some(Evidence {
                chunk_id: row.chunk_id,
                chunk_set_id: row.chunk_set_id,
                source_id: row.source_id,
                source_type,
                source_version: row.source_version,
                title: row.title,
                text: row.text,
                locations: row.locations.0,
                score,
            })
        })
        .take(MAX_EVIDENCE)
        .collect()
}
