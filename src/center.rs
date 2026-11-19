//! Read-only catalogue discovery and page navigation through the optional center feature.
//!
//! Mutating installation and repository operations require the ordinary user interface.
//! Searches preserve the visible query; opening a page routes a normal application message.

#[cfg(not(test))]
use enzui_agent::center::{
    AppFunction, AppFunctionRegistry, AppMetadata, CenterHost, CenterProvision, ConsentScope,
    FunctionRisk, FunctionSpec,
};
use enzui_agent::center::{CenterFailure, CenterOutcome, CenterTurn, ConnectorEvent};
#[cfg(not(test))]
use serde::Deserialize;
use serde_json::{Value, json};

use super::*;

// The app id the center credential is enrolled under.
#[cfg(not(test))]
pub(crate) const CENTER_APP_ID: &str = "io.github.enzui.Store";
// The most rows one listing returns.
pub(crate) const LIST_LIMIT_MAX: usize = 50;

// What an admitted call asks Store to do.
#[cfg_attr(
    test,
    expect(
        dead_code,
        reason = "Calls are constructed by the native desktop connector"
    )
)]
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum CenterCall {
    SearchApps { query: String, limit: usize },
    ListInstalled,
    ShowApp { app_id: String },
}

#[cfg(not(test))]
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchArgs {
    query: String,
    #[serde(default)]
    limit: Option<usize>,
}

#[cfg(not(test))]
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AppArgs {
    app_id: String,
}

#[cfg(not(test))]
fn object(properties: Value, required: &[&str]) -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": required,
        "properties": properties,
    })
}

#[cfg(not(test))]
fn row_schema() -> Value {
    object(
        json!({
            "app_id": {"type": "string"},
            "name": {"type": "string"},
            "summary": {"type": "string"},
            "installed": {"type": "boolean"}
        }),
        &["app_id", "name", "summary", "installed"],
    )
}

// Store's registered functions.
#[cfg(not(test))]
pub(crate) fn registry() -> AppFunctionRegistry<CenterCall> {
    let search = FunctionSpec::new(
        "search_apps",
        "Search apps",
        "Searches the catalogue by relevance: id, name, summary and whether it is installed.",
        object(
            json!({
                "query": {"type": "string", "minLength": 1, "maxLength": 100},
                "limit": {"type": "integer", "minimum": 1, "maximum": LIST_LIMIT_MAX}
            }),
            &["query"],
        ),
        object(
            json!({"apps": {"type": "array", "items": row_schema()}}),
            &["apps"],
        ),
        FunctionRisk::ReadOnly,
        vec![
            ConsentScope::Once,
            ConsentScope::Session,
            ConsentScope::Always,
        ],
    );
    let installed = FunctionSpec::new(
        "list_installed",
        "List installed apps",
        "Lists installed apps: id, name and version.",
        object(json!({}), &[]),
        object(
            json!({"apps": {"type": "array", "items": object(json!({
                "app_id": {"type": "string"},
                "name": {"type": "string"},
                "version": {"type": "string"}
            }), &["app_id", "name", "version"])}}),
            &["apps"],
        ),
        FunctionRisk::SensitiveRead,
        vec![ConsentScope::Once, ConsentScope::Session],
    );
    let show = FunctionSpec::new(
        "show_app",
        "Show an app's page",
        "Opens the detail page of an app the catalogue or the installed list knows.",
        object(
            json!({"app_id": {"type": "string", "minLength": 1, "maxLength": 255}}),
            &["app_id"],
        ),
        object(json!({"app_id": {"type": "string"}}), &["app_id"]),
        FunctionRisk::Mutating,
        vec![ConsentScope::Once, ConsentScope::Session],
    );
    AppFunctionRegistry::from_functions(vec![
        AppFunction::typed(search, |args: SearchArgs| CenterCall::SearchApps {
            query: args.query,
            limit: args.limit.unwrap_or(20).clamp(1, LIST_LIMIT_MAX),
        })
        .expect("search_apps spec"),
        AppFunction::new(installed, |_| CenterCall::ListInstalled).expect("list_installed spec"),
        AppFunction::typed(show, |args: AppArgs| CenterCall::ShowApp {
            app_id: args.app_id,
        })
        .expect("show_app spec"),
    ])
    .expect("the store registry is valid")
}

#[cfg(not(test))]
pub(crate) fn metadata() -> AppMetadata {
    AppMetadata {
        id: CENTER_APP_ID.into(),
        name: "Store".into(),
        description: "Flatpak app store".into(),
        version: env!("CARGO_PKG_VERSION").into(),
    }
}

impl Store {
    // Connect to enzui-center when this build opted in and a credential is
    // enrolled for [`CENTER_APP_ID`]; otherwise stay unexposed.
    #[cfg(not(test))]
    pub(crate) fn start_center(&mut self) {
        let Some(provision) = CenterProvision::discover(CENTER_APP_ID) else {
            return;
        };
        match CenterHost::spawn(provision, metadata(), registry()) {
            Ok(host) => self.center = Some(host),
            Err(error) => eprintln!("store-app: center connector disabled: {error}"),
        }
    }

    pub(crate) fn center_subscription(&self) -> Subscription<Message> {
        self.center
            .as_ref()
            .map_or_else(Subscription::none, |host| {
                host.subscription(Message::Center)
            })
    }

    // Final shutdown: refuse new calls, cancel live ones, join the connector.
    pub(crate) fn shutdown_center(&mut self) {
        if let Some(host) = self.center.take() {
            host.shutdown();
        }
    }

    pub(crate) fn center_event(&mut self, event: ConnectorEvent) -> Task<Message> {
        let Some(host) = self.center.as_mut() else {
            return Task::none();
        };
        // These handlers answer in this turn; subsequent UI work belongs to the app.
        let CenterTurn::Call(call, request) = host.receive_with_ends(event).turn else {
            return Task::none();
        };
        let (task, outcome) = self.apply_center_call(request);
        if let Some(host) = self.center.as_mut() {
            host.answer(call, outcome);
        }
        task
    }

    fn apply_center_call(&mut self, request: CenterCall) -> (Task<Message>, CenterOutcome) {
        match request {
            CenterCall::SearchApps { query, limit } => {
                let rows = crate::model::search_rows_from(
                    self.catalog_candidates().into_iter(),
                    &query,
                    crate::model::SearchFilters::default(),
                    &self.installed_ids(),
                    &self.update_ids(),
                );
                let apps: Vec<Value> = rows
                    .iter()
                    .take(limit)
                    .map(|entry| {
                        json!({
                            "app_id": entry.id,
                            "name": entry.name,
                            "summary": entry.summary,
                            "installed": entry.installed,
                        })
                    })
                    .collect();
                (Task::none(), Ok(json!({ "apps": apps })))
            }
            CenterCall::ListInstalled => {
                let apps: Vec<Value> = self
                    .installed
                    .iter()
                    .take(LIST_LIMIT_MAX * 4)
                    .map(|entry| {
                        json!({
                            "app_id": entry.id,
                            "name": entry.name,
                            "version": entry.version,
                        })
                    })
                    .collect();
                (Task::none(), Ok(json!({ "apps": apps })))
            }
            CenterCall::ShowApp { app_id } => {
                let known = self
                    .catalog_candidates()
                    .iter()
                    .any(|entry| entry.id == app_id);
                if !known {
                    return (
                        Task::none(),
                        Err(CenterFailure::new("not_found", "no known app has that id")),
                    );
                }
                let task = self.update(Message::OpenAppDetail(app_id.clone()));
                (task, Ok(json!({ "app_id": app_id })))
            }
        }
    }
}
