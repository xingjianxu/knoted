//! Dashboard aggregation uses one read-only repeatable snapshot and the shared
//! task calendar rules. This endpoint never queues jobs or sends notifications.
mod repository;
use crate::{
    context::AppContext,
    notes::service::Note,
    tasks::{
        dates::date_boundary,
        service::{DomainError, Task, TaskStatus},
    },
};
use serde::Serialize;

#[derive(Serialize)]
pub struct DashboardCounts {
    pub notes: i64,
    pub tasks_open: i64,
    pub tasks_due_today: i64,
    pub projects_active: i64,
    /// Registered present sources; unhashed scan observations are not sources.
    pub files: i64,
}
#[derive(Default, Serialize)]
pub struct DashboardTasksByStatus {
    pub todo: i64,
    pub in_progress: i64,
    pub done: i64,
    pub cancelled: i64,
}
#[derive(Serialize)]
pub struct DashboardSummary {
    pub counts: DashboardCounts,
    pub tasks_by_status: DashboardTasksByStatus,
    pub recent_tasks: Vec<Task>,
    pub recent_notes: Vec<Note>,
    pub due_tasks: Vec<Task>,
    pub job_failures: i64,
    pub indexing_enabled: bool,
}
pub struct GetDashboard;

pub async fn summary(ctx: &AppContext, _: GetDashboard) -> Result<DashboardSummary, DomainError> {
    let now = ctx.now();
    let today = now.with_timezone(&chrono_tz::Asia::Shanghai).date_naive();
    let mut tx = ctx.pool().begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *tx)
        .await?;
    let recent_notes = repository::recent_notes(&mut tx).await?;
    let (notes, projects_active, job_failures, files) = repository::counts(&mut tx).await?;
    let tasks = repository::tasks(&mut tx).await?;
    let mut tasks_by_status = DashboardTasksByStatus::default();
    let mut tasks_due_today = 0;
    let mut due = Vec::new();
    for task in &tasks {
        match task.status {
            TaskStatus::Todo => tasks_by_status.todo += 1,
            TaskStatus::InProgress => tasks_by_status.in_progress += 1,
            TaskStatus::Done => tasks_by_status.done += 1,
            TaskStatus::Cancelled => tasks_by_status.cancelled += 1,
        }
        if matches!(task.status, TaskStatus::Done | TaskStatus::Cancelled) {
            continue;
        }
        if let Some(value) = &task.due {
            // Validation and deadline ordering use the exact T35 boundary.
            let boundary = date_boundary(value, true)?;
            // The original deadline is valid, but its calendar day's midnight
            // need not exist (e.g. Shanghai 1919-04-13). Read its date without
            // imposing a second, unrelated local-time mapping constraint.
            let date = chrono::NaiveDate::parse_from_str(&value.date, "%Y-%m-%d")
                .map_err(|_| DomainError::InvalidField("start/due"))?;
            if date == today {
                tasks_due_today += 1;
            }
            if date <= today {
                due.push((boundary, task.id, task.clone()));
            }
        }
    }
    due.sort_by_key(|(boundary, id, _)| (*boundary, *id));
    let result = DashboardSummary {
        counts: DashboardCounts {
            notes,
            projects_active,
            tasks_open: tasks_by_status.todo + tasks_by_status.in_progress,
            tasks_due_today,
            files,
        },
        tasks_by_status,
        recent_tasks: tasks.into_iter().take(10).collect(),
        recent_notes,
        due_tasks: due.into_iter().take(10).map(|(_, _, task)| task).collect(),
        job_failures,
        indexing_enabled: false,
    };
    tx.commit().await?;
    Ok(result)
}
