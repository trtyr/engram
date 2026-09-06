//! 路由注册与中间件装配。

pub mod auth_api;
pub mod codegraph_api;
pub mod health;
pub mod jobs_api;
pub mod llm_api;
pub mod memory_api;
pub mod migrate_api;
pub mod project_api;
pub mod search_api;
pub mod skills_api;
pub mod todos_api;
pub mod wiki_api;
pub mod wiki_docs_api;

use crate::state::AppState;
use axum::middleware::from_fn_with_state;
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    info(title = "engram API", version = env!("CARGO_PKG_VERSION"),
        description = "单用户 AI 长期记忆平台。平台即工具：AI 通过本 API 操纵记忆。"),
    paths(
        health::health, health::ready,
        auth_api::login_handler, auth_api::status, auth_api::init_account,
        auth_api::change_credentials, auth_api::list_sessions, auth_api::revoke_session,
        auth_api::revoke_others, auth_api::username,
        jobs_api::list_jobs, jobs_api::get_job, jobs_api::get_job_events, jobs_api::revive_job,
        llm_api::create_provider, llm_api::list_providers, llm_api::test_provider,
        llm_api::update_provider, llm_api::delete_provider, llm_api::reencrypt_providers,
        llm_api::get_routing, llm_api::put_routing, llm_api::suggest_routing, llm_api::usage,
        llm_api::create_api_key_handler, llm_api::list_api_keys, llm_api::revoke_api_key,
        llm_api::batch_revoke_api_keys, llm_api::fetch_models,
        crate::mcp_admin::settings_mcp, crate::mcp_admin::settings_mcp_update,
        memory_api::write_session, memory_api::list_sessions, memory_api::get_session,
        memory_api::erase_session, memory_api::append_session, memory_api::import_session, memory_api::void_session, memory_api::trigger_distill, memory_api::purge_agent, memory_api::export_memory,
        memory_api::list_atoms, memory_api::create_atom, memory_api::update_atom,
        memory_api::list_scenarios, memory_api::get_scenario,
        memory_api::get_persona, memory_api::persona_edit, memory_api::persona_history,
        memory_api::persona_rollback, memory_api::atom_revisions,
        memory_api::search, memory_api::context, memory_api::embedding_status, memory_api::reembed_memory,
        memory_api::rhythm_heartbeat, memory_api::rhythm_status,
        memory_api::timeline,
        memory_api::list_entities, memory_api::entity_graph, memory_api::search_entities_handler, memory_api::create_entity,
        memory_api::batch_entities, memory_api::export_entities,
        memory_api::get_entity, memory_api::update_entity, memory_api::delete_entity,
        memory_api::attach_atom, memory_api::detach_atom, memory_api::merge_entity, memory_api::entity_revisions,
        memory_api::list_entity_relations, memory_api::create_entity_relation, memory_api::delete_entity_relation,
        search_api::search,
        wiki_docs_api::submit_url, wiki_docs_api::upload, wiki_docs_api::list_documents,
        wiki_docs_api::get_document, wiki_docs_api::document_chunks,
        wiki_docs_api::delete_document, wiki_docs_api::reembed, wiki_docs_api::search,
        wiki_api::ingest, wiki_api::list_pages, wiki_api::get_page, wiki_api::put_page,
        wiki_api::graph, wiki_api::lint, wiki_api::apply_proposal, wiki_api::search,
 wiki_api::rebuild_links,
        wiki_api::list_proposals,
        wiki_api::get_purpose, wiki_api::set_purpose,
        wiki_api::list_reviews, wiki_api::resolve_review,
        wiki_api::archive_query, wiki_api::list_sources, wiki_api::delete_source,
        wiki_api::insights, wiki_api::dismiss_insight, wiki_api::reset_insights,
        codegraph_api::register_project, codegraph_api::list_projects,
        codegraph_api::get_project, codegraph_api::delete_project,
        codegraph_api::index_project, codegraph_api::sync_project,
        codegraph_api::query, codegraph_api::status, codegraph_api::graph,
        migrate_api::export_bundle, migrate_api::import_bundle, migrate_api::pull,
        todos_api::list_todos, todos_api::create_todo, todos_api::get_todo,
        todos_api::update_todo, todos_api::delete_todo, todos_api::export_todos,
        project_api::list_types, project_api::create_project, project_api::list_projects,
        project_api::get_project, project_api::update_project, project_api::delete_project,
        project_api::batch_delete_projects,
        project_api::add_location, project_api::get_location, project_api::update_location, project_api::delete_location,
        project_api::add_doc, project_api::get_doc, project_api::update_doc, project_api::delete_doc,
        skills_api::list_skills, skills_api::create_skill, skills_api::import_skills,
        skills_api::export_skills, skills_api::get_skill, skills_api::update_skill,
        skills_api::delete_skill, skills_api::list_revisions, skills_api::restore_revision,
        skills_api::import_transfer,
        skills_api::list_files, skills_api::get_file, skills_api::put_file, skills_api::delete_file,
        skills_api::bundle,
    ),
)]
pub(crate) struct ApiDoc;

pub fn router(state: AppState) -> Router {
    let public = Router::new()
        .route("/health", get(health::health))
        .route("/ready", get(health::ready))
        .route("/openapi.json", get(openapi_json))
        .route("/auth/login", post(auth_api::login_handler))
        .route("/auth/status", get(auth_api::status))
        .route("/auth/username", get(auth_api::username))
        .route("/auth/init", post(auth_api::init_account))
        .route(
            "/auth/account",
            axum::routing::put(auth_api::change_credentials),
        )
        .route(
            "/auth/sessions",
            get(auth_api::list_sessions).post(auth_api::revoke_others),
        )
        .route(
            "/auth/sessions/{id}",
            axum::routing::delete(auth_api::revoke_session),
        );

    let authed = Router::new()
        // MCP（用户记忆域工具面）：nest 在 authed 内 → 复用 Bearer 中间件，
        // 每个 JSON-RPC 请求独立认证（key 吊销即刻生效，会话保活不能豁免）；
        // gate 在 Bearer 之内、MCP 之前——服务总开关关闭时对已认证客户端也 503
        .merge(
            Router::new()
                .nest_service("/mcp", engram_mcp::service(state.clone()))
                .route_layer(from_fn_with_state(state.clone(), engram_mcp::gate)),
        )
        .route("/jobs", get(jobs_api::list_jobs))
        .route("/jobs/{id}", get(jobs_api::get_job))
        .route("/jobs/{id}/events", get(jobs_api::get_job_events))
        .route("/jobs/{id}/revive", post(jobs_api::revive_job))
        .route(
            "/settings/llm/providers",
            post(llm_api::create_provider).get(llm_api::list_providers),
        )
        .route(
            "/settings/llm/providers/models",
            post(llm_api::fetch_models),
        )
        .route(
            "/settings/llm/providers/{id}",
            put(llm_api::update_provider).delete(llm_api::delete_provider),
        )
        .route(
            "/settings/llm/providers/re-encrypt",
            post(llm_api::reencrypt_providers),
        )
        .route(
            "/settings/llm/providers/{id}/test",
            post(llm_api::test_provider),
        )
        .route(
            "/settings/llm/routing",
            get(llm_api::get_routing).put(llm_api::put_routing),
        )
        .route(
            "/settings/llm/routing/suggest",
            post(llm_api::suggest_routing),
        )
        .route(
            "/settings/api-keys",
            post(llm_api::create_api_key_handler).get(llm_api::list_api_keys),
        )
        .route(
            "/settings/api-keys/{id}/revoke",
            post(llm_api::revoke_api_key),
        )
        .route(
            "/settings/api-keys/batch-revoke",
            post(llm_api::batch_revoke_api_keys),
        )
        .route(
            "/settings/mcp",
            get(crate::mcp_admin::settings_mcp).put(crate::mcp_admin::settings_mcp_update),
        )
        .route("/llm/usage", get(llm_api::usage))
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
        // 节律（memory-rhythm）：外部 cron 的心跳与状态
        .route(
            "/memory/rhythm/heartbeat",
            post(memory_api::rhythm_heartbeat),
        )
        .route("/memory/rhythm/status", get(memory_api::rhythm_status))
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
        .route("/search", post(search_api::search))
        // 文档知识并入 Wiki 前缀（/wiki/documents、/wiki/upload）
        .route(
            "/wiki/documents",
            post(wiki_docs_api::submit_url).get(wiki_docs_api::list_documents),
        )
        .route("/wiki/upload", post(wiki_docs_api::upload))
        // search 先于 {id}，避免 "search" 被当作 id 解析
        .route("/wiki/documents/search", post(wiki_docs_api::search))
        .route(
            "/wiki/documents/{id}",
            get(wiki_docs_api::get_document).delete(wiki_docs_api::delete_document),
        )
        .route(
            "/wiki/documents/{id}/chunks",
            get(wiki_docs_api::document_chunks),
        )
        .route(
            "/wiki/documents/{id}/re-embed",
            post(wiki_docs_api::reembed),
        )
        .route("/wiki/ingest", post(wiki_api::ingest))
        .route("/wiki/pages", get(wiki_api::list_pages))
        .route(
            "/wiki/pages/{slug}",
            get(wiki_api::get_page)
                .put(wiki_api::put_page)
                .delete(wiki_api::delete_page),
        )
        .route("/wiki/graph", get(wiki_api::graph))
        .route("/wiki/lint", post(wiki_api::lint))
        .route("/wiki/links/rebuild", post(wiki_api::rebuild_links))
        .route("/wiki/proposals", get(wiki_api::list_proposals))
        .route("/wiki/proposals/apply", post(wiki_api::apply_proposal))
        .route("/wiki/search", post(wiki_api::search))
        .route(
            "/wiki/purpose",
            get(wiki_api::get_purpose).put(wiki_api::set_purpose),
        )
        .route("/wiki/reviews", get(wiki_api::list_reviews))
        .route("/wiki/reviews/{id}/resolve", post(wiki_api::resolve_review))
        .route("/wiki/queries/archive", post(wiki_api::archive_query))
        .route("/wiki/sources", get(wiki_api::list_sources))
        .route(
            "/wiki/sources/{id}",
            axum::routing::delete(wiki_api::delete_source),
        )
        .route("/wiki/insights", post(wiki_api::insights))
        .route("/wiki/insights/dismiss", post(wiki_api::dismiss_insight))
        .route("/wiki/insights/reset", post(wiki_api::reset_insights))
        .route(
            "/codegraph/projects",
            post(codegraph_api::register_project).get(codegraph_api::list_projects),
        )
        .route("/codegraph/status", get(codegraph_api::status))
        .route(
            "/codegraph/projects/{id}",
            get(codegraph_api::get_project).delete(codegraph_api::delete_project),
        )
        .route("/codegraph/projects/{id}/graph", get(codegraph_api::graph))
        .route(
            "/codegraph/projects/{id}/index",
            post(codegraph_api::index_project),
        )
        .route(
            "/codegraph/projects/{id}/sync",
            post(codegraph_api::sync_project),
        )
        .route("/codegraph/projects/{id}/query", post(codegraph_api::query))
        // 项目记忆域：types 与 batch-delete 先于 {id}，避免被当作 id 解析
        .route("/projects/types", get(project_api::list_types))
        .route(
            "/projects/batch-delete",
            post(project_api::batch_delete_projects),
        )
        .route(
            "/projects",
            post(project_api::create_project).get(project_api::list_projects),
        )
        .route(
            "/projects/{id}",
            get(project_api::get_project)
                .put(project_api::update_project)
                .delete(project_api::delete_project),
        )
        .route("/projects/{id}/locations", post(project_api::add_location))
        .route(
            "/projects/{id}/locations/{loc_id}",
            get(project_api::get_location)
                .put(project_api::update_location)
                .delete(project_api::delete_location),
        )
        .route("/projects/{id}/docs", post(project_api::add_doc))
        .route(
            "/projects/{id}/docs/{doc_id}",
            get(project_api::get_doc)
                .put(project_api::update_doc)
                .delete(project_api::delete_doc),
        )
        // 技能域：import/export 先于 {slug}，避免被当作 slug 解析
        .route("/skills/import", post(skills_api::import_skills))
        .route("/skills/{slug}/files", get(skills_api::list_files))
        .route("/skills/{slug}/bundle", get(skills_api::bundle))
        .route(
            "/skills/{slug}/file",
            get(skills_api::get_file)
                .put(skills_api::put_file)
                .delete(skills_api::delete_file),
        )
        .route("/skills/export", get(skills_api::export_skills))
        .route("/skills/import-transfer", post(skills_api::import_transfer))
        .route(
            "/todos",
            get(todos_api::list_todos).post(todos_api::create_todo),
        )
        .route(
            "/todos/{id}",
            get(todos_api::get_todo)
                .put(todos_api::update_todo)
                .delete(todos_api::delete_todo),
        )
        .route("/todos/export", get(todos_api::export_todos))
        .route("/migrate/export", get(migrate_api::export_bundle))
        .route("/migrate/import", post(migrate_api::import_bundle))
        .route("/migrate/pull", post(migrate_api::pull))
        .route(
            "/skills",
            post(skills_api::create_skill).get(skills_api::list_skills),
        )
        .route(
            "/skills/{slug}",
            get(skills_api::get_skill)
                .put(skills_api::update_skill)
                .delete(skills_api::delete_skill),
        )
        .route("/skills/{slug}/revisions", get(skills_api::list_revisions))
        .route(
            "/skills/{slug}/revisions/{rev_id}/restore",
            post(skills_api::restore_revision),
        );

    Router::new()
        .merge(public)
        .merge(authed.layer(from_fn_with_state(state.clone(), crate::auth::bearer_auth)))
        // SPA 静态资源兜底（API 路由未命中时 → web/dist）
        .fallback_service(axum::routing::any(crate::web_assets::static_handler))
        .with_state(state)
}

async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}

/// OpenAPI 文档（程序化访问，openapi-dump bin 用）。
pub fn openapi() -> utoipa::openapi::OpenApi {
    ApiDoc::openapi()
}
