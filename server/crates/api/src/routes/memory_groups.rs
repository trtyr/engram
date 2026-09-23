use super::*;

/// `memory_routes` 路由组 0（纯注册表，拆自原函数）。
pub(super) fn memory_routes_group0() -> Router<AppState> {
    Router::new()
        .route("/memory/sessions/import", post(memory_api::import_session))
        .route(
            "/memory/sessions",
            post(memory_api::write_session).get(memory_api::list_sessions),
        )
        .route(
            "/memory/sessions/{id}",
            get(memory_api::get_session).delete(memory_api::erase_session),
        )
        .route(
            "/memory/sessions/{id}/append",
            post(memory_api::append_session),
        )
        .route("/memory/sessions/{id}/void", post(memory_api::void_session))
        .route(
            "/memory/sessions/{id}/restore",
            post(memory_api::restore_session),
        )
        .route(
            "/memory/sessions/batch-restore",
            post(memory_api::batch_restore_sessions),
        )
        .route(
            "/memory/sessions/batch-erase",
            post(memory_api::batch_erase_sessions),
        )
        .route("/memory/purge", post(memory_api::purge_agent))
        .route("/memory/export", get(memory_api::export_memory))
        .route("/memory/distill", post(memory_api::trigger_distill))
        .route(
            "/memory/atoms",
            get(memory_api::list_atoms).post(memory_api::create_atom),
        )
        .route(
            "/memory/atoms/{id}",
            axum::routing::patch(memory_api::update_atom),
        )
}

/// `memory_routes` 路由组 1（纯注册表，拆自原函数）。
pub(super) fn memory_routes_group1() -> Router<AppState> {
    Router::new()
        .route(
            "/memory/atoms/{id}/revisions",
            axum::routing::get(memory_api::atom_revisions),
        )
        .route("/memory/scenarios", get(memory_api::list_scenarios))
        .route("/memory/scenarios/{id}", get(memory_api::get_scenario))
        .route(
            "/memory/persona",
            get(memory_api::get_persona).patch(memory_api::persona_edit),
        )
        .route("/memory/persona/history", get(memory_api::persona_history))
        .route(
            "/memory/persona/rollback",
            post(memory_api::persona_rollback),
        )
        .route("/memory/search", post(memory_api::search))
        .route("/memory/context", get(memory_api::context))
        .route(
            "/memory/embeddings/status",
            get(memory_api::embedding_status),
        )
        .route("/memory/reembed", post(memory_api::reembed_memory))
        .route("/memory/kv", get(memory_api::list_kv))
}

/// `memory_routes` 路由组 2（纯注册表，拆自原函数）。
pub(super) fn memory_routes_group2() -> Router<AppState> {
    Router::new()
        .route("/memory/kv/{key}", get(memory_api::get_kv))
        .route("/memory/timeline", get(memory_api::timeline))
        // 实体（记忆星系）：graph/search 路由先于 {id}，避免 "graph"/"search" 被当作 id
        .route("/memory/entities/graph", get(memory_api::entity_graph))
        .route(
            "/memory/entities/search",
            get(memory_api::search_entities_handler),
        )
        .route("/memory/entities/batch", post(memory_api::batch_entities))
        .route("/memory/entities/export", get(memory_api::export_entities))
        .route(
            "/memory/entities",
            get(memory_api::list_entities).post(memory_api::create_entity),
        )
        .route(
            "/memory/entities/{id}",
            get(memory_api::get_entity)
                .patch(memory_api::update_entity)
                .delete(memory_api::delete_entity),
        )
        .route(
            "/memory/entities/{id}/atoms/{atom_id}",
            post(memory_api::attach_atom).delete(memory_api::detach_atom),
        )
        .route(
            "/memory/entities/{id}/merge",
            post(memory_api::merge_entity),
        )
        .route(
            "/memory/entities/{id}/revisions",
            get(memory_api::entity_revisions),
        )
        .route(
            "/memory/entities/{id}/relations",
            get(memory_api::list_entity_relations).post(memory_api::create_entity_relation),
        )
        .route(
            "/memory/entities/{id}/relations/{rid}",
            delete(memory_api::delete_entity_relation),
        )
}
