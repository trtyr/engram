//! `wiki_api` 的实现切片（架构治理 2026-09-21：自 wiki_api.rs 纯搬移，零行为变化）。

use super::*;

pub(crate) fn svc(state: &AppState) -> WikiService {
    WikiService::new(state.pool.clone(), state.registry()).with_llm(state.llm())
}
