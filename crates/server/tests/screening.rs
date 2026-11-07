//! Seam: the router and `jobs::publish_due`, with a fake model client, for screening
//! (design.md, Submission and screening, steps 3 and 4): verdicts become statuses,
//! batches are decided as one, every call's cost is logged, and the daily caps hold
//! entries back. Also the Messages API request and reply shapes.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::fake_models::FakeModels;
use common::{DELIMITER, TestApp, entry, field, id_in, test_app_with};
use filedrawer_server::jobs::publish_due;
use filedrawer_server::screening::{
    Answer, Model, ModelError, ModelRequest, Subject, Usage, parse_reply, request_body,
    status_error,
};
use serde_json::json;
use time::{Duration, OffsetDateTime};

const DOLLAR: i64 = 1_000_000_000;

async fn app_with(models: &FakeModels) -> TestApp {
    test_app_with(models.screener(), DOLLAR).await
}

/// Submits `text` and ends its hold; returns its ID and the status the reply gave.
async fn publish(app: &TestApp, text: &str) -> (String, String) {
    let held = app.post("/entries", text.to_string()).await;
    assert_eq!(held.status, StatusCode::CREATED, "{}", held.body);
    let id = id_in(&held.body);
    let placed = app.post(&format!("/e/{id}/publish"), "").await;
    assert_eq!(placed.status, StatusCode::OK, "{}", placed.body);
    let status = field(&placed.body, "status").unwrap().to_string();
    (id, status)
}

fn marked(marker: &str, front: &str) -> String {
    entry(&format!("Steering {marker}"), front)
}

#[tokio::test]
async fn an_entry_that_passes_is_published_and_its_cost_logged() {
    let models = FakeModels::default();
    let app = app_with(&models).await;
    let (id, status) = publish(&app, &entry("Steering at 14", "")).await;
    assert_eq!(status, "public");
    assert_eq!(models.asked(), vec![Model::Haiku]);
    assert_eq!(app.get(&format!("/e/{id}")).await.status, StatusCode::OK);
    let (model, cost): (String, i64) =
        sqlx::query_as("SELECT model, cost_nano_usd FROM screenings")
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(model, "claude-haiku-5-5");
    // 1,000 input tokens at $0.10 and 200 output at $0.50 per million.
    assert_eq!(cost, 1_000 * 100 + 200 * 500);
}

#[tokio::test]
async fn an_off_topic_entry_is_rejected_and_its_author_reads_why() {
    let models = FakeModels::default();
    let app = app_with(&models).await;
    let (id, status) = publish(&app, &marked("FAKE-OFFTOPIC", "slug: cake\n")).await;
    assert_eq!(status, "rejected");
    assert_eq!(
        app.get(&format!("/e/{id}")).await.status,
        StatusCode::NOT_FOUND
    );
    let own = app.get_as_author(&format!("/e/{id}")).await;
    assert_eq!(own.status, StatusCode::OK, "{}", own.body);
    assert_eq!(field(&own.body, "status"), Some("rejected"));
    let (_, below) = own.body.split_once(DELIMITER).unwrap();
    assert!(
        below.contains("reason: It is about cooking, not AI safety."),
        "{}",
        own.body
    );
    // Its slug is free again, so the author can resubmit with it.
    let again = app
        .post("/entries", entry("Steering, rewritten", "slug: cake\n"))
        .await;
    assert_eq!(again.status, StatusCode::CREATED, "{}", again.body);
}

/// A test app with staging's smoke-test account set: its own account, or `other`.
async fn smoke_app(models: &FakeModels, other: Option<&str>) -> TestApp {
    let mut app = app_with(models).await;
    app.state.smoke_account = Some(other.map_or_else(|| app.account.clone(), str::to_string));
    app.router = filedrawer_server::app(app.state.clone());
    app
}

#[tokio::test]
async fn the_smoke_accounts_entries_publish_unscreened_and_say_so() {
    // CI's smoke test runs after every deploy; it must not call the model each time.
    let models = FakeModels::default();
    let app = smoke_app(&models, None).await;
    let (id, status) = publish(&app, &marked("FAKE-OFFTOPIC FAKE-HAZARD", "")).await;
    assert_eq!(status, "public");
    assert_eq!(models.asked(), vec![]);
    let page = app.get(&format!("/e/{id}")).await;
    assert!(
        page.body.contains("staging smoke account: not screened"),
        "{}",
        page.body
    );
    let calls: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM screenings")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(calls, 0);
}

#[tokio::test]
async fn the_smoke_accounts_batches_and_series_skip_the_model_too() {
    let models = FakeModels::default();
    let app = smoke_app(&models, None).await;
    let reply = batch(
        &app,
        &[
            marked("FAKE-OFFTOPIC", "slug: one\n"),
            marked("FAKE-SPAM", "slug: two\n"),
        ],
    )
    .await;
    let batch_id = reply["id"].as_str().unwrap();
    let published = app.post(&format!("/batches/{batch_id}/publish"), "").await;
    assert_eq!(published.status, StatusCode::OK, "{}", published.body);
    for id in common::batch_member_ids(&reply) {
        let page = app.get(&format!("/e/{id}")).await;
        assert_eq!(page.status, StatusCode::OK, "{id} isn't public");
        assert!(
            page.body.contains("staging smoke account: not screened"),
            "{}",
            page.body
        );
    }
    let series = app
        .post(
            "/series",
            common::series::series_doc("Smoke series FAKE-OFFTOPIC", "Shared setup."),
        )
        .await;
    assert_eq!(series.status, StatusCode::CREATED, "{}", series.body);
    assert_eq!(models.asked(), vec![]);
}

#[tokio::test]
async fn another_accounts_rejected_entry_stays_rejected_beside_a_smoke_account() {
    let models = FakeModels::default();
    let app = smoke_app(&models, Some("a-someone")).await;
    let (_, status) = publish(&app, &marked("FAKE-OFFTOPIC", "")).await;
    assert_eq!(status, "rejected");
}

#[tokio::test]
async fn spam_and_a_download_piped_into_a_shell_are_rejected() {
    let models = FakeModels::default();
    let app = app_with(&models).await;
    let (_, spam) = publish(&app, &marked("FAKE-SPAM", "")).await;
    assert_eq!(spam, "rejected");
    let (id, pipe) = publish(&app, &marked("FAKE-PIPE", "")).await;
    assert_eq!(pipe, "rejected");
    let own = app.get_as_author(&format!("/e/{id}")).await;
    assert!(
        own.body.contains("pipes a download into a shell"),
        "{}",
        own.body
    );
}

#[tokio::test]
async fn a_possible_hazard_goes_to_sonnet_and_passes_only_if_sonnet_clears_it() {
    let models = FakeModels::default();
    let app = app_with(&models).await;
    let (_, cleared) = publish(&app, &marked("FAKE-HAZARD", "")).await;
    assert_eq!(cleared, "public");
    assert_eq!(models.asked(), vec![Model::Haiku, Model::Sonnet]);

    let (id, held) = publish(&app, &marked("FAKE-HAZARD FAKE-SONNET-HAZARD", "")).await;
    assert_eq!(held, "review");
    assert_eq!(
        app.get(&format!("/e/{id}")).await.status,
        StatusCode::NOT_FOUND
    );
    let own = app.get_as_author(&format!("/e/{id}")).await;
    assert_eq!(field(&own.body, "status"), Some("review"), "{}", own.body);

    // Haiku being unsure of a hazard asks Sonnet too.
    let (_, unsure) = publish(&app, &marked("FAKE-HAZARD-UNSURE FAKE-SONNET-HAZARD", "")).await;
    assert_eq!(unsure, "review");
}

#[tokio::test]
async fn sonnet_unsure_of_a_hazard_or_quoting_nothing_passes_with_no_public_note() {
    let models = FakeModels::default();
    let app = app_with(&models).await;
    for markers in [
        "FAKE-HAZARD FAKE-SONNET-UNSURE",
        "FAKE-HAZARD FAKE-SONNET-NOQUOTE",
    ] {
        let (id, status) = publish(&app, &marked(markers, "")).await;
        assert_eq!(status, "public", "{markers}");
        let page = app.get(&format!("/e/{id}")).await;
        assert!(!page.body.contains("quoted passage"), "{}", page.body);
        assert!(!page.body.contains("screening note"), "{}", page.body);
    }
}

#[tokio::test]
async fn a_refusal_counts_as_a_possible_hazard() {
    let models = FakeModels::default();
    let app = app_with(&models).await;
    let (_, cleared) = publish(&app, &marked("FAKE-REFUSE", "")).await;
    assert_eq!(cleared, "public", "Sonnet read it and cleared it");
    let (_, held) = publish(&app, &marked("FAKE-REFUSE FAKE-SONNET-REFUSE", "")).await;
    assert_eq!(held, "review");
}

#[tokio::test]
async fn haiku_unsure_of_topic_or_spam_passes_without_asking_sonnet() {
    let models = FakeModels::default();
    let app = app_with(&models).await;
    for markers in ["FAKE-UNSURE FAKE-SONNET-OFFTOPIC", "FAKE-SPAM-UNSURE"] {
        let (_, status) = publish(&app, &marked(markers, "")).await;
        assert_eq!(status, "public", "{markers}");
    }
    assert_eq!(models.asked(), vec![Model::Haiku, Model::Haiku]);
}

#[tokio::test]
async fn sonnets_topic_call_is_final_when_it_looks_for_a_hazard() {
    let models = FakeModels::default();
    let app = app_with(&models).await;
    let (_, failed) = publish(&app, &marked("FAKE-HAZARD FAKE-SONNET-OFFTOPIC", "")).await;
    assert_eq!(failed, "rejected");
}

#[tokio::test]
async fn a_doubt_about_the_outcome_shows_on_the_card_as_a_screening_note() {
    let models = FakeModels::default();
    let app = app_with(&models).await;
    let (id, status) = publish(&app, &marked("FAKE-DOUBT", "")).await;
    assert_eq!(status, "public");
    let read = app.get(&format!("/e/{id}")).await;
    assert!(read.body.contains("screening note"), "{}", read.body);
    assert!(
        read.body.contains("The body reports an effect."),
        "{}",
        read.body
    );
    let cards = app.get("/cards.txt").await.body;
    assert!(cards.contains("The body reports an effect."), "{cards}");
}

#[tokio::test]
async fn an_unreadable_reply_is_retried_and_after_three_goes_to_review() {
    let models = FakeModels::default();
    let app = app_with(&models).await;
    let (id, status) = publish(&app, &marked("FAKE-GARBLED", "")).await;
    assert_eq!(status, "screening", "it waits for the next try");
    let now = OffsetDateTime::now_utc();
    publish_due(&app.state, now).await.unwrap();
    let own = app.get_as_author(&format!("/e/{id}")).await;
    assert_eq!(
        field(&own.body, "status"),
        Some("screening"),
        "{}",
        own.body
    );
    publish_due(&app.state, now).await.unwrap();
    let own = app.get_as_author(&format!("/e/{id}")).await;
    assert_eq!(field(&own.body, "status"), Some("review"), "{}", own.body);
    assert_eq!(models.asked().len(), 3);
}

#[tokio::test]
async fn entries_past_the_days_cap_wait_for_the_next_day() {
    let models = FakeModels::default();
    // One call costs 200,000 billionths; the account's 10% share of this cap is less.
    let app = test_app_with(models.screener(), 1_000_000).await;
    let (_, first) = publish(&app, &entry("First", "")).await;
    assert_eq!(first, "public");
    let (id, second) = publish(&app, &entry("Second", "")).await;
    assert_eq!(second, "screening");
    assert_eq!(models.asked().len(), 1);
    let today = OffsetDateTime::now_utc();
    publish_due(&app.state, today).await.unwrap();
    assert_eq!(
        app.get(&format!("/e/{id}")).await.status,
        StatusCode::NOT_FOUND
    );
    publish_due(&app.state, today + Duration::days(1))
        .await
        .unwrap();
    assert_eq!(app.get(&format!("/e/{id}")).await.status, StatusCode::OK);
}

#[tokio::test]
async fn one_accounts_share_of_the_cap_doesnt_hold_back_another() {
    let models = FakeModels::default();
    // A 10% share is 200,000 billionths, one call: the first account's second entry
    // waits, and another account's first entry still goes through.
    let app = test_app_with(models.screener(), 2_000_000).await;
    publish(&app, &entry("First", "")).await;
    let (_, second) = publish(&app, &entry("Second", "")).await;
    assert_eq!(second, "screening");
    let other = app.other_token().await;
    let held = app
        .post_as("/entries", Some(&other), entry("Theirs", ""))
        .await;
    let id = id_in(&held.body);
    let placed = app
        .post_as(&format!("/e/{id}/publish"), Some(&other), "")
        .await;
    assert_eq!(
        field(&placed.body, "status"),
        Some("public"),
        "{}",
        placed.body
    );
}

async fn batch(app: &TestApp, entries: &[String]) -> serde_json::Value {
    batch_with(app, json!([]), entries).await
}

/// Sends a batch declaring `runs`, which must be accepted, and returns the reply.
async fn batch_with(
    app: &TestApp,
    runs: serde_json::Value,
    entries: &[String],
) -> serde_json::Value {
    let reply = app
        .send(
            Request::post("/entries/batch")
                .header("authorization", format!("Bearer {}", app.token))
                .header("content-type", "application/json")
                .header("accept", "application/json")
                .body(Body::from(
                    json!({"runs": runs, "entries": entries}).to_string(),
                ))
                .unwrap(),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    serde_json::from_str(&reply.body).unwrap()
}

#[tokio::test]
async fn one_failing_member_sends_the_whole_batch_back() {
    let models = FakeModels::default();
    let app = app_with(&models).await;
    let reply = batch(
        &app,
        &[
            marked("FAKE-OFFTOPIC", "slug: bad\n"),
            entry("Fine", "slug: fine\n"),
            entry("Also fine", "slug: also\n"),
        ],
    )
    .await;
    let ids = common::batch_member_ids(&reply);
    let batch_id = reply["id"].as_str().unwrap();
    let published = app.post(&format!("/batches/{batch_id}/publish"), "").await;
    assert_eq!(published.status, StatusCode::OK, "{}", published.body);
    for id in &ids {
        assert!(
            published.body.contains(&format!("entry: {id} rejected")),
            "{}",
            published.body
        );
    }
    // The member after the failed one wasn't screened at all.
    assert_eq!(models.asked().len(), 1);
    let own = app.get_as_author(&format!("/e/{}", ids[1])).await;
    assert!(
        own.body.contains(&format!(
            "another entry in its batch, {}, was rejected",
            ids[0]
        )),
        "{}",
        own.body
    );
    // No slug stays reserved.
    let again = app
        .post("/entries", entry("Fine again", "slug: fine\n"))
        .await;
    assert_eq!(again.status, StatusCode::CREATED, "{}", again.body);
}

#[tokio::test]
async fn a_member_failing_after_one_went_to_review_leaves_that_one_to_the_owner() {
    // Reaching review doesn't guard the account (only the owner's reject or hold
    // does), so the second member is screened and fails. The first stays in review
    // for the owner to see (the owner's decision, 2026-11-06).
    let models = FakeModels::default();
    let app = app_with(&models).await;
    let reply = batch(
        &app,
        &[
            marked("FAKE-HAZARD FAKE-SONNET-HAZARD", "slug: risky\n"),
            marked("FAKE-OFFTOPIC", "slug: bad\n"),
        ],
    )
    .await;
    let ids = common::batch_member_ids(&reply);
    let batch_id = reply["id"].as_str().unwrap();
    let published = app.post(&format!("/batches/{batch_id}/publish"), "").await;
    assert_eq!(published.status, StatusCode::OK, "{}", published.body);
    assert!(
        published
            .body
            .contains(&format!("entry: {} review", ids[0])),
        "{}",
        published.body
    );
    assert!(
        published
            .body
            .contains(&format!("entry: {} rejected", ids[1])),
        "{}",
        published.body
    );
}

#[tokio::test]
async fn a_likely_hazard_needs_both_a_category_and_a_quote() {
    let models = FakeModels::default();
    let app = app_with(&models).await;
    for markers in [
        "FAKE-HAZARD FAKE-SONNET-NOCATEGORY",
        "FAKE-HAZARD FAKE-SONNET-EMPTYQUOTE",
    ] {
        let (_, status) = publish(&app, &marked(markers, "")).await;
        assert_eq!(status, "public", "{markers}");
    }
}

#[tokio::test]
async fn a_batch_waits_while_a_member_is_in_review() {
    let models = FakeModels::default();
    let app = app_with(&models).await;
    let reply = batch(
        &app,
        &[
            entry("Fine", "slug: fine\n"),
            marked("FAKE-HAZARD FAKE-SONNET-HAZARD", "slug: risky\n"),
        ],
    )
    .await;
    let ids = common::batch_member_ids(&reply);
    let batch_id = reply["id"].as_str().unwrap();
    let published = app.post(&format!("/batches/{batch_id}/publish"), "").await;
    assert!(
        published
            .body
            .contains(&format!("entry: {} screening", ids[0])),
        "{}",
        published.body
    );
    assert!(
        published
            .body
            .contains(&format!("entry: {} review", ids[1])),
        "{}",
        published.body
    );
    assert_eq!(
        app.get(&format!("/e/{}", ids[0])).await.status,
        StatusCode::NOT_FOUND
    );
    // The worker doesn't screen the passed member again.
    publish_due(&app.state, OffsetDateTime::now_utc())
        .await
        .unwrap();
    assert_eq!(
        models.asked(),
        vec![Model::Haiku, Model::Haiku, Model::Sonnet]
    );
}

#[tokio::test]
async fn the_hours_end_screens_through_the_worker() {
    let models = FakeModels::default();
    let app = app_with(&models).await;
    let held = app.post("/entries", marked("FAKE-OFFTOPIC", "")).await;
    let id = id_in(&held.body);
    publish_due(
        &app.state,
        OffsetDateTime::now_utc() + Duration::minutes(61),
    )
    .await
    .unwrap();
    let own = app.get_as_author(&format!("/e/{id}")).await;
    assert_eq!(field(&own.body, "status"), Some("rejected"), "{}", own.body);
}

#[tokio::test]
async fn a_series_the_screen_fails_is_refused() {
    let models = FakeModels::default();
    let app = app_with(&models).await;
    let reply = app
        .post(
            "/series",
            "---\ntitle: Cakes\n---\n\nFAKE-OFFTOPIC recipes.\n",
        )
        .await;
    assert_eq!(
        reply.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        reply.body
    );
    assert!(reply.body.contains("cooking"), "{}", reply.body);
    let logged: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM screenings")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(logged, 1);
}

/// Keywords the API refuses in a structured-output schema (structured-outputs.md,
/// "JSON Schema limitations": numeric and string constraints, and array constraints
/// beyond `minItems`, which is allowed only as 0 or 1).
const UNSUPPORTED: &[&str] = &[
    "minimum",
    "maximum",
    "exclusiveMinimum",
    "exclusiveMaximum",
    "multipleOf",
    "minLength",
    "maxLength",
    "maxItems",
    "uniqueItems",
    "contains",
    "prefixItems",
];
/// The string formats it supports.
const FORMATS: &[&str] = &[
    "date-time",
    "time",
    "date",
    "duration",
    "email",
    "hostname",
    "uri",
    "ipv4",
    "ipv6",
    "uuid",
];
/// Its complexity limits, across a request's schemas ("Schema complexity limits").
const MAX_OPTIONAL: usize = 24;
const MAX_UNION: usize = 16;

/// Where `schema` breaks a limit the Messages API documents for structured outputs
/// (https://platform.claude.com/docs/en/build-with-claude/structured-outputs.md),
/// such as the enum rule behind staging's 400 of 2026-11-06: "Enum value
/// 'working_attack' does not match declared type '['string', 'null']'". It doesn't
/// check recursion or `pattern`'s regex limits; our schema uses neither.
fn schema_problems(schema: &serde_json::Value) -> Vec<String> {
    let mut tally = Tally::default();
    tally.walk(schema, "schema");
    if tally.optional > MAX_OPTIONAL {
        let optional = tally.optional;
        tally.problems.push(format!(
            "{optional} optional properties; the limit is {MAX_OPTIONAL}"
        ));
    }
    if tally.union > MAX_UNION {
        let union = tally.union;
        tally.problems.push(format!(
            "{union} union-typed properties; the limit is {MAX_UNION}"
        ));
    }
    tally.problems
}

/// What a walk over a schema found: its problems, and the counts the limits cap.
#[derive(Default)]
struct Tally {
    problems: Vec<String>,
    optional: usize,
    union: usize,
}

impl Tally {
    fn walk(&mut self, schema: &serde_json::Value, path: &str) {
        // Each enum needs one declared type, and every value must be of that type.
        if let Some(values) = schema.get("enum").and_then(|values| values.as_array()) {
            let fits = |value: &serde_json::Value| match schema["type"].as_str() {
                Some("string") => value.is_string(),
                Some("boolean") => value.is_boolean(),
                Some("integer") => value.is_i64() || value.is_u64(),
                Some("number") => value.is_number(),
                Some("null") => value.is_null(),
                _ => false,
            };
            for value in values.iter().filter(|value| !fits(value)) {
                let declared = &schema["type"];
                self.problems
                    .push(format!("{path}: {value} under type {declared}"));
            }
        }
        for keyword in UNSUPPORTED
            .iter()
            .filter(|keyword| schema.get(**keyword).is_some())
        {
            self.problems
                .push(format!("{path}: {keyword} isn't supported"));
        }
        if let Some(min) = schema.get("minItems")
            && *min != 0
            && *min != 1
        {
            self.problems
                .push(format!("{path}: minItems {min}; only 0 or 1 is supported"));
        }
        if let Some(format) = schema.get("format").and_then(|format| format.as_str())
            && !FORMATS.contains(&format)
        {
            self.problems
                .push(format!("{path}: format {format} isn't supported"));
        }
        if let Some(reference) = schema.get("$ref").and_then(|r| r.as_str())
            && !reference.starts_with('#')
        {
            self.problems
                .push(format!("{path}: external $ref {reference}"));
        }
        for (n, branch) in schema["allOf"].as_array().into_iter().flatten().enumerate() {
            if branch.get("$ref").is_some() {
                self.problems.push(format!(
                    "{path}.allOf[{n}]: allOf with $ref isn't supported"
                ));
            }
        }
        if schema["type"].is_array() || schema.get("anyOf").is_some() {
            self.union += 1;
        }
        // An object, by its type or by having properties, must refuse others.
        let object = schema["type"] == "object"
            || schema["type"]
                .as_array()
                .is_some_and(|types| types.contains(&json!("object")))
            || schema.get("properties").is_some();
        if object && schema.get("additionalProperties") != Some(&json!(false)) {
            self.problems
                .push(format!("{path}: additionalProperties must be false"));
        }
        let required: Vec<&str> = schema["required"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|name| name.as_str())
            .collect();
        for (name, property) in schema["properties"].as_object().into_iter().flatten() {
            if !required.contains(&name.as_str()) {
                self.optional += 1;
            }
            self.walk(property, &format!("{path}.{name}"));
        }
        if schema.get("items").is_some() {
            self.walk(&schema["items"], &format!("{path}[]"));
        }
        for key in ["anyOf", "allOf"] {
            for (n, branch) in schema[key].as_array().into_iter().flatten().enumerate() {
                self.walk(branch, &format!("{path}.{key}[{n}]"));
            }
        }
        for key in ["$defs", "definitions"] {
            for (name, defined) in schema[key].as_object().into_iter().flatten() {
                self.walk(defined, &format!("{path}.{key}.{name}"));
            }
        }
    }
}

#[tokio::test]
async fn every_schema_sent_keeps_to_the_apis_documented_limits() {
    let models = FakeModels::default();
    let screener = models.screener();
    screener.screen_as("an entry", Subject::Submission).await;
    screener.screen_as("some code", Subject::Code).await;
    let schemas = models.schemas();
    assert_eq!(schemas.len(), 2);
    for schema in &schemas {
        assert_eq!(schema_problems(schema), Vec::<String>::new());
    }
}

/// A one-property object schema around `property`.
fn with_property(property: serde_json::Value) -> serde_json::Value {
    json!({
        "type": "object", "additionalProperties": false, "required": ["a"],
        "properties": {"a": property},
    })
}

/// An object schema with `count` string properties typed as `property`, `required`
/// listing the first `required` of them.
fn with_properties(
    count: usize,
    required: usize,
    property: serde_json::Value,
) -> serde_json::Value {
    let names: Vec<String> = (0..count).map(|n| format!("p{n}")).collect();
    let properties: serde_json::Map<String, serde_json::Value> = names
        .iter()
        .map(|name| (name.clone(), property.clone()))
        .collect();
    json!({
        "type": "object", "additionalProperties": false,
        "required": names[..required], "properties": properties,
    })
}

#[test]
fn the_schema_checker_catches_each_documented_limit() {
    let string = json!({"type": "string"});
    let nullable = json!({"type": ["string", "null"]});
    let cases = [
        (
            with_property(json!({"type": ["string", "null"], "enum": ["x", null]})),
            "under type",
        ),
        (
            json!({"type": "object", "required": [], "properties": {}}),
            "additionalProperties",
        ),
        (
            with_property(json!({"type": "integer", "minimum": 0})),
            "minimum",
        ),
        (
            with_property(json!({"type": "string", "maxLength": 9})),
            "maxLength",
        ),
        (
            with_property(json!({"type": "array", "items": string, "minItems": 2})),
            "minItems",
        ),
        (
            with_property(json!({"$ref": "https://example.com/schema.json"})),
            "external $ref",
        ),
        (with_properties(25, 0, string.clone()), "optional"),
        (with_properties(17, 17, nullable.clone()), "union"),
        (
            with_property(
                json!({"type": "object", "properties": {"b": {"type": "integer", "multipleOf": 2}}}),
            ),
            "multipleOf",
        ),
        (
            with_property(json!({"type": "array", "items": string, "uniqueItems": true})),
            "uniqueItems",
        ),
        (
            with_property(json!({"type": "string", "format": "phone"})),
            "format phone",
        ),
        (
            with_property(json!({"allOf": [{"$ref": "#/$defs/x"}]})),
            "allOf with $ref",
        ),
        // An object by its properties, or by a type list holding "object".
        (
            with_property(json!({"properties": {"b": string}})),
            "schema.a: additionalProperties",
        ),
        (
            with_property(json!({"type": ["object", "null"], "properties": {}})),
            "schema.a: additionalProperties",
        ),
        // Inside anyOf, allOf and $defs.
        (
            with_property(json!({"anyOf": [{"type": "integer", "minimum": 1}, {"type": "null"}]})),
            "schema.a.anyOf[0]: minimum",
        ),
        (
            with_property(json!({"allOf": [{"type": "integer", "multipleOf": 3}]})),
            "schema.a.allOf[0]: multipleOf",
        ),
        (
            json!({
                "type": "object", "additionalProperties": false, "required": [],
                "properties": {}, "$defs": {"d": {"type": "string", "maxLength": 3}},
            }),
            "schema.$defs.d: maxLength",
        ),
        (
            with_properties(
                17,
                17,
                json!({"anyOf": [{"type": "string"}, {"type": "null"}]}),
            ),
            "union",
        ),
    ];
    for (schema, named) in cases {
        let problems = schema_problems(&schema);
        assert!(
            problems.iter().any(|problem| problem.contains(named)),
            "{named}: {problems:?} for {schema}"
        );
    }
    // Within the limits: 24 optional, 16 union-typed, an enum of its own type.
    assert_eq!(
        schema_problems(&with_properties(24, 0, string)),
        Vec::<String>::new()
    );
    assert_eq!(
        schema_problems(&with_properties(16, 16, nullable)),
        Vec::<String>::new()
    );
    assert_eq!(
        schema_problems(&with_property(
            json!({"type": "string", "enum": ["x", "y"]})
        )),
        Vec::<String>::new()
    );
}

#[test]
fn the_request_asks_for_structured_output_at_the_models_effort() {
    let request = ModelRequest {
        model: Model::Haiku,
        system: "rules".into(),
        user: "<x>text</x>".into(),
        schema: json!({"type": "object"}),
    };
    let body = request_body(&request);
    assert_eq!(body["model"], "claude-haiku-5-5");
    assert_eq!(body["thinking"], json!({"type": "adaptive"}));
    assert_eq!(body["output_config"]["effort"], "low");
    assert_eq!(
        body["output_config"]["format"],
        json!({"type": "json_schema", "schema": {"type": "object"}})
    );
    assert_eq!(body["system"], "rules");
    assert_eq!(body["messages"][0]["content"], "<x>text</x>");
    assert!(body.get("temperature").is_none());
    let sonnet = request_body(&ModelRequest {
        model: Model::Sonnet,
        ..request
    });
    assert_eq!(sonnet["model"], "claude-sonnet-5-5");
    assert_eq!(sonnet["output_config"]["effort"], "medium");
}

#[test]
fn a_refused_request_carries_the_apis_reason_for_the_log() {
    // The reply staging got from 2026-11-06 20:04 UTC on, with its log line.
    let refused = status_error(
        400,
        r#"{"type":"error","error":{"type":"invalid_request_error","message":"output_config.format.schema: Invalid schema: Enum value 'working_attack' does not match declared type '['string', 'null']'"},"request_id":"req_011CgnYtYoEHDeFPJDwZxntD"}"#,
    );
    assert_eq!(
        refused.to_string(),
        "the API answered 400: invalid_request_error: output_config.format.schema: Invalid schema: Enum value 'working_attack' does not match declared type '['string', 'null']'"
    );
    // A proxy's page or an empty body holds no reason.
    for body in ["<html><body>502 Bad Gateway</body></html>", ""] {
        assert_eq!(
            status_error(502, body).to_string(),
            "the API answered 502: no error in the reply"
        );
    }
}

#[test]
fn the_apis_reason_is_kept_to_one_short_line() {
    let message = format!("first line\nsecond {}", "é".repeat(1_000));
    let body = json!({"type": "error", "error": {"type": "api_error", "message": message}});
    let ModelError::Status { status, message } = status_error(500, &body.to_string()) else {
        panic!("not a status error");
    };
    assert_eq!(status, 500);
    assert!(
        message.starts_with("api_error: first line second éé"),
        "{message}"
    );
    assert!(!message.contains('\n'));
    assert!(
        message.chars().count() <= 300,
        "{} chars",
        message.chars().count()
    );
    assert!(message.ends_with('…'), "{message}");
}

#[test]
fn a_reply_gives_its_answer_a_refusal_or_why_it_cant_be_used_and_always_its_usage() {
    let usage = json!({"input_tokens": 12, "output_tokens": 34});
    let answered = parse_reply(&json!({
        "stop_reason": "end_turn",
        "content": [{"type": "thinking", "thinking": ""}, {"type": "text", "text": "{\"spam\": \"no\"}"}],
        "usage": usage,
    }));
    assert_eq!(answered.answer, Answer::Json(json!({"spam": "no"})));
    assert_eq!(
        answered.usage,
        Usage {
            input_tokens: 12,
            output_tokens: 34
        }
    );
    let refused = parse_reply(&json!({"stop_reason": "refusal", "content": [], "usage": usage}));
    assert_eq!(refused.answer, Answer::Refused);
    assert_eq!(refused.usage.output_tokens, 34);
    let cut_off = parse_reply(&json!({"stop_reason": "max_tokens", "content": [], "usage": usage}));
    assert!(matches!(cut_off.answer, Answer::Unreadable(_)));
    assert_eq!(
        cut_off.usage.output_tokens, 34,
        "a cut-off reply still cost tokens"
    );
    let garbled = parse_reply(
        &json!({"stop_reason": "end_turn", "content": [{"type": "text", "text": "not json"}], "usage": usage}),
    );
    assert!(matches!(garbled.answer, Answer::Unreadable(_)));
}

#[test]
fn haiku_costs_more_per_token_past_100k_input_tokens() {
    let small = Usage {
        input_tokens: 100_000,
        output_tokens: 0,
    };
    let large = Usage {
        input_tokens: 100_001,
        output_tokens: 0,
    };
    assert_eq!(Model::Haiku.cost_nano_usd(small), 100_000 * 100);
    assert_eq!(Model::Haiku.cost_nano_usd(large), 100_001 * 500);
    assert_eq!(
        Model::Sonnet.cost_nano_usd(Usage {
            input_tokens: 1,
            output_tokens: 1
        }),
        12_000
    );
}

#[tokio::test]
async fn an_entry_being_screened_isnt_screened_again_by_the_worker() {
    let models = FakeModels::default();
    let app = app_with(&models).await;
    let held = app.post("/entries", marked("FAKE-SLOW", "")).await;
    let id = id_in(&held.body);
    let router = app.router.clone();
    let token = app.token.clone();
    let publishing = tokio::spawn(async move {
        use tower::ServiceExt;
        router
            .oneshot(
                Request::post(format!("/e/{id}/publish"))
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
    });
    models.wait_until_asked(1).await;
    let worker = tokio::spawn({
        let state = app.state.clone();
        async move {
            publish_due(&state, OffsetDateTime::now_utc())
                .await
                .unwrap()
        }
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    models.release();
    publishing.await.unwrap();
    worker.await.unwrap();
    assert_eq!(models.asked(), vec![Model::Haiku], "screened once");
}

#[tokio::test]
async fn an_unreadable_reply_still_logs_its_cost() {
    let models = FakeModels::default();
    let app = app_with(&models).await;
    let (_, status) = publish(&app, &marked("FAKE-GARBLED", "")).await;
    assert_eq!(status, "screening");
    let logged: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM screenings")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(logged, 1);
}

#[tokio::test]
async fn an_outage_doesnt_send_entries_to_review() {
    let models = FakeModels::default();
    let app = app_with(&models).await;
    let (id, _) = publish(&app, &marked("FAKE-ERROR", "")).await;
    for _ in 0..4 {
        publish_due(&app.state, OffsetDateTime::now_utc())
            .await
            .unwrap();
    }
    let own = app.get_as_author(&format!("/e/{id}")).await;
    assert_eq!(
        field(&own.body, "status"),
        Some("screening"),
        "{}",
        own.body
    );
}

#[tokio::test]
async fn the_worker_places_an_entry_left_screened_but_unplaced() {
    let models = FakeModels::default();
    let app = app_with(&models).await;
    let held = app.post("/entries", entry("Left behind", "")).await;
    let id = id_in(&held.body);
    // As a crash between storing the verdict and placing the entry would leave it.
    sqlx::query("UPDATE entries SET status = 'screening', screen_verdict = 'pass', hold_until = NULL WHERE id = ?")
        .bind(&id)
        .execute(&app.pool)
        .await
        .unwrap();
    publish_due(&app.state, OffsetDateTime::now_utc())
        .await
        .unwrap();
    assert_eq!(app.get(&format!("/e/{id}")).await.status, StatusCode::OK);
    assert!(models.asked().is_empty());
}

#[tokio::test]
async fn a_refused_series_puts_the_models_reasons_below_the_delimiter() {
    let models = FakeModels::default();
    let app = app_with(&models).await;
    let reply = app
        .post(
            "/series",
            "---\ntitle: Cakes\n---\n\nFAKE-OFFTOPIC recipes.\n",
        )
        .await;
    let (above, below) = reply.body.split_once(DELIMITER).expect("a delimiter");
    assert!(!above.contains("cooking"), "{}", reply.body);
    assert!(
        below.contains("reason: It is about cooking, not AI safety."),
        "{}",
        reply.body
    );
}

fn run(description: &str) -> serde_json::Value {
    json!({"id": "sweep-1", "description": description, "n": 520,
           "commit": "0123456789abcdef0123456789abcdef01234567", "code_state": "committed",
           "route": "weights", "dates": ["2026-10-20", "2026-10-21"]})
}

/// Sends a batch declaring `runs` and returns its ID.
async fn batch_with_runs(app: &TestApp, runs: serde_json::Value, entries: &[String]) -> String {
    batch_with(app, runs, entries).await["id"]
        .as_str()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn a_run_declaration_is_screened_with_the_entries_that_use_it() {
    let models = FakeModels::default();
    let app = app_with(&models).await;
    let id = batch_with_runs(
        &app,
        json!([run("Sweep over layers 10 to 18. FAKE-OFFTOPIC")]),
        &[
            entry("Fine", "slug: fine\nuses_runs: [sweep-1]\n"),
            entry("Also fine", "slug: also\n"),
        ],
    )
    .await;
    let published = app.post(&format!("/batches/{id}/publish"), "").await;
    assert_eq!(published.status, StatusCode::OK, "{}", published.body);
    assert_eq!(
        published.body.matches(" rejected").count(),
        2,
        "{}",
        published.body
    );
    let screened = models.screened();
    assert!(
        screened[0].contains("sweep-1") && screened[0].contains("Sweep over layers 10 to 18"),
        "{}",
        screened[0]
    );
}

#[tokio::test]
async fn a_clean_run_declaration_passes_with_its_entries() {
    let models = FakeModels::default();
    let app = app_with(&models).await;
    let id = batch_with_runs(
        &app,
        json!([run("Sweep over layers 10 to 18, 520 prompts each.")]),
        &[entry("Fine", "slug: fine\nuses_runs: [sweep-1]\n")],
    )
    .await;
    let published = app.post(&format!("/batches/{id}/publish"), "").await;
    assert!(published.body.contains(" public"), "{}", published.body);
}

#[tokio::test]
async fn a_run_cited_from_a_later_entry_is_screened_again_with_it() {
    let models = FakeModels::default();
    let app = app_with(&models).await;
    let id = batch_with_runs(
        &app,
        json!([run("Sweep over layers 10 to 18, 520 prompts each.")]),
        &[entry("Fine", "slug: fine\nuses_runs: [sweep-1]\n")],
    )
    .await;
    app.post(&format!("/batches/{id}/publish"), "").await;
    let later = app
        .post("/entries", entry("Later", "uses_runs: [sweep-1]\n"))
        .await;
    assert_eq!(later.status, StatusCode::CREATED, "{}", later.body);
    app.post(&format!("/e/{}/publish", id_in(&later.body)), "")
        .await;
    let screened = models.screened();
    assert!(
        screened
            .last()
            .unwrap()
            .contains("Sweep over layers 10 to 18")
    );
}

#[tokio::test]
async fn the_screen_test_account_isnt_held_to_an_accounts_share_but_is_to_the_day() {
    // Each fake call costs 200,000; a cap of 1,000,000 gives an account 100,000.
    let models = FakeModels::default();
    let mut app = test_app_with(models.screener(), 1_000_000).await;
    app.state.screen_test_account = Some(app.account.clone());
    app.router = filedrawer_server::app(app.state.clone());
    let other = app.other_token().await;
    let other_publishes = async |title: &str| {
        let held = app
            .post_as("/entries", Some(&other), entry(title, ""))
            .await;
        let id = id_in(&held.body);
        let placed = app
            .post_as(&format!("/e/{id}/publish"), Some(&other), "")
            .await;
        field(&placed.body, "status").unwrap().to_string()
    };
    // Another account spends its 100,000 share on one entry, then waits.
    assert_eq!(other_publishes("Other one").await, "public");
    assert_eq!(other_publishes("Other two").await, "screening");
    // The screen-test account goes on past its share, to the day's 1,000,000.
    for n in 0..4 {
        let (_, status) = publish(
            &app,
            &entry(&format!("Steering {n}"), &format!("slug: s{n}\n")),
        )
        .await;
        assert_eq!(status, "public", "entry {n}");
    }
    let (_, status) = publish(&app, &entry("Steering 4", "slug: s4\n")).await;
    assert_eq!(status, "screening");
}
