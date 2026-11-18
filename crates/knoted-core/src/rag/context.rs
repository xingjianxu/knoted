//! Generation budget and durable source-version dependencies.

use std::collections::{HashSet, VecDeque};

use serde::{Deserialize, Serialize};
use sqlx::PgConnection;
use uuid::Uuid;

use crate::{context::AppContext, search::meili::SourceType};

pub const CONTEXT_TOKENS: usize = 32 * 1024;
pub const OUTPUT_TOKENS: usize = 4 * 1024;
const MESSAGE_FRAMING_TOKENS: usize = 16;
const RUN_FRAMING_TOKENS: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SourceDependency {
    pub source_type: SourceType,
    pub source_id: Uuid,
    pub source_version: i64,
    pub chunk_set_id: Uuid,
    pub visibility_generation: i64,
}

#[derive(Debug)]
pub struct HistoryTurn {
    pub question: String,
    pub answer: String,
    /// Includes the dependencies inherited when this turn was generated.
    pub dependencies: Vec<SourceDependency>,
}

#[derive(Debug)]
pub struct ContextEvidence {
    pub chunk_id: Uuid,
    pub text: String,
    pub dependency: SourceDependency,
}

#[derive(Debug)]
pub struct ContextInput {
    pub system: String,
    pub question: String,
    pub history: Vec<HistoryTurn>,
    /// Ranked from strongest to weakest.
    pub evidence: Vec<ContextEvidence>,
    pub unavailable: HashSet<SourceDependency>,
    /// Provider’s configured input-side maximum.
    pub input_limit: usize,
}

#[derive(Debug)]
pub struct Context {
    pub system: String,
    pub question: String,
    pub history: Vec<HistoryTurn>,
    pub evidence: Vec<ContextEvidence>,
    pub dependencies: Vec<SourceDependency>,
    pub input_tokens: usize,
    pub max_output_tokens: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum ContextError {
    #[error("context_too_large")]
    TooLarge,
    #[error("source_changed")]
    SourceChanged,
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

pub fn build_context(
    input: ContextInput,
    count: impl Fn(&str) -> usize,
) -> Result<Context, ContextError> {
    let limit = input.input_limit.min(CONTEXT_TOKENS - OUTPUT_TOKENS);
    let tokens = |text: &str| count(text).saturating_add(MESSAGE_FRAMING_TOKENS);
    let baseline = tokens(&input.system)
        .saturating_add(tokens(&input.question))
        .saturating_add(RUN_FRAMING_TOKENS);
    if baseline > limit {
        return Err(ContextError::TooLarge);
    }
    let mut history: VecDeque<HistoryTurn> = input
        .history
        .into_iter()
        .filter(|round| {
            !round
                .dependencies
                .iter()
                .any(|dep| input.unavailable.contains(dep))
        })
        .collect();
    let mut evidence: Vec<ContextEvidence> = input
        .evidence
        .into_iter()
        .filter(|item| !input.unavailable.contains(&item.dependency))
        .collect();
    let mut total = baseline;
    for item in &evidence {
        total = total.saturating_add(tokens(&item.text));
    }
    for turn in &history {
        total = total
            .saturating_add(tokens(&turn.question))
            .saturating_add(tokens(&turn.answer));
    }
    while total > limit {
        if let Some(turn) = history.pop_front() {
            total = total
                .saturating_sub(tokens(&turn.question))
                .saturating_sub(tokens(&turn.answer));
        } else if evidence.len() > 1 {
            total = total.saturating_sub(tokens(&evidence.pop().expect("remaining evidence").text));
        } else {
            return Err(ContextError::TooLarge);
        }
    }

    let mut seen = HashSet::new();
    let dependencies = evidence
        .iter()
        .map(|item| item.dependency)
        .chain(
            history
                .iter()
                .flat_map(|turn| turn.dependencies.iter().copied()),
        )
        .filter(|dep| seen.insert(*dep))
        .collect();

    Ok(Context {
        system: input.system,
        question: input.question,
        history: history.into(),
        evidence,
        dependencies,
        input_tokens: total,
        max_output_tokens: OUTPUT_TOKENS,
    })
}

pub async fn unavailable_sources(
    tx: &mut PgConnection,
    dependencies: &[SourceDependency],
) -> Result<HashSet<SourceDependency>, ContextError> {
    let sets: Vec<_> = dependencies.iter().map(|dep| dep.chunk_set_id).collect();
    let current: HashSet<_> = capture_sources(tx, &sets).await?.into_iter().collect();
    Ok(dependencies
        .iter()
        .filter(|dependency| !current.contains(dependency))
        .copied()
        .collect())
}

pub async fn verify_generation_sources(
    tx: &mut PgConnection,
    dependencies: &[SourceDependency],
) -> Result<(), ContextError> {
    if dependencies.is_empty() {
        return Ok(());
    }
    // Mutations bump the same rows in their own transaction. Hold a share lock
    // through this short state transition, never while streaming the model.
    let kinds: Vec<_> = dependencies
        .iter()
        .map(|dep| dep.source_type.as_str())
        .collect();
    let ids: Vec<_> = dependencies.iter().map(|dep| dep.source_id).collect();
    sqlx::query(
        "INSERT INTO rag_source_visibility (source_type,source_id,generation)
         SELECT DISTINCT kind,source_id,0 FROM unnest($1::text[],$2::uuid[]) AS ids(kind,source_id)
         ORDER BY kind,source_id ON CONFLICT DO NOTHING",
    )
    .bind(&kinds)
    .bind(&ids)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "SELECT source_type,source_id FROM rag_source_visibility
         WHERE (source_type,source_id) IN (SELECT * FROM unnest($1::text[],$2::uuid[]))
         ORDER BY source_type,source_id FOR SHARE",
    )
    .bind(kinds)
    .bind(ids)
    .fetch_all(&mut *tx)
    .await?;
    if unavailable_sources(tx, dependencies).await?.is_empty() {
        Ok(())
    } else {
        Err(ContextError::SourceChanged)
    }
}

pub async fn invalidate_source_dependencies(
    ctx: &AppContext,
    source_type: SourceType,
    source_id: Uuid,
) -> Result<(), ContextError> {
    sqlx::query("SELECT rag_bump_visibility($1,$2)")
        .bind(source_type.as_str())
        .bind(source_id)
        .execute(ctx.pool())
        .await?;
    Ok(())
}

/// Read the durable visibility generation at the point evidence is selected.
pub async fn capture_sources(
    tx: &mut PgConnection,
    chunk_set_ids: &[Uuid],
) -> Result<Vec<SourceDependency>, ContextError> {
    if chunk_set_ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows: Vec<(Uuid, String, Uuid, i64, i64)> = sqlx::query_as(
        "SELECT s.id,s.source_type,s.source_id,s.source_version,COALESCE(g.generation,0)
         FROM rag_chunk_sets s
         JOIN current_rag_sources live USING (source_type,source_id,source_version,extraction_id)
         LEFT JOIN rag_source_visibility g ON g.source_type=s.source_type AND g.source_id=s.source_id
         WHERE s.id=ANY($1)",
    )
    .bind(chunk_set_ids)
    .fetch_all(tx)
    .await?;
    rows.into_iter()
        .map(
            |(chunk_set_id, kind, source_id, source_version, visibility_generation)| {
                Ok(SourceDependency {
                    source_type: SourceType::parse(&kind).ok_or(ContextError::SourceChanged)?,
                    source_id,
                    source_version,
                    chunk_set_id,
                    visibility_generation,
                })
            },
        )
        .collect()
}

/// Save the actual dependencies fed to the provider, including inherited ones.
pub async fn persist_dependencies(
    tx: &mut PgConnection,
    run_id: Uuid,
    dependencies: &[SourceDependency],
) -> Result<(), ContextError> {
    verify_generation_sources(tx, dependencies).await?;
    for dep in dependencies {
        sqlx::query(
            "INSERT INTO generation_source_dependencies
              (run_id, chunk_set_id, source_type, source_id, source_version, visibility_generation)
             VALUES ($1,$2,$3,$4,$5,$6)",
        )
        .bind(run_id)
        .bind(dep.chunk_set_id)
        .bind(dep.source_type.as_str())
        .bind(dep.source_id)
        .bind(dep.source_version)
        .bind(dep.visibility_generation)
        .execute(&mut *tx)
        .await?;
    }
    Ok(())
}

pub async fn load_dependencies(
    tx: &mut PgConnection,
    run_id: Uuid,
) -> Result<Vec<SourceDependency>, ContextError> {
    let rows: Vec<(Uuid, String, Uuid, i64, i64)> = sqlx::query_as(
        "SELECT chunk_set_id,source_type,source_id,source_version,visibility_generation
         FROM generation_source_dependencies WHERE run_id=$1 ORDER BY chunk_set_id",
    )
    .bind(run_id)
    .fetch_all(tx)
    .await?;
    rows.into_iter()
        .map(
            |(chunk_set_id, kind, source_id, source_version, visibility_generation)| {
                Ok(SourceDependency {
                    source_type: SourceType::parse(&kind).ok_or(ContextError::SourceChanged)?,
                    source_id,
                    source_version,
                    chunk_set_id,
                    visibility_generation,
                })
            },
        )
        .collect()
}
