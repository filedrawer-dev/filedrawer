//! Series for agents (design.md, Series): `POST /series`, `POST /series/<id>/join`,
//! `POST /series/<id>/versions`, and `GET /series/<id>` or `/series/<id>@<version>`,
//! a page for a browser, and plain text for every client with `.txt` after it.
//! The creator makes a series public from the account page, not here.

use analyzer::{SeriesText, read_series};
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use serde_json::json;

use crate::auth::{TokenAccount, optional_token_account};
use crate::error::AppError;
use crate::markdown;
use crate::pages::{SeriesPage, fields_above, page};
use crate::queue::Screened;
use crate::respond::{DELIMITER, Format, Reading, author_name, one_line};
use crate::screening::Verdict;
use crate::series::{self, Series, Version};
use crate::{AppState, allowance, jobs, queue, time};

/// Who wrote something, after the account ID on a line: "by <name>", with the name
/// last so nothing in it can pass for the account, or "account deleted".
fn byline(display_name: Option<&str>) -> String {
    match display_name {
        Some(name) => format!("by {}", one_line(name)),
        None => author_name(None),
    }
}

/// Reads and screens a series document, or refuses it with every reason. It is
/// screened at once, within the day's caps and the account's allowance, and its cost
/// logged against `account` (design.md, Series: the owner's decision, 2026-11-06).
async fn accept_series(
    state: &AppState,
    account: &TokenAccount,
    versioning: Option<&str>,
    body: &Bytes,
) -> Result<SeriesText, AppError> {
    let refused = |reasons| AppError::Refused {
        status: StatusCode::UNPROCESSABLE_ENTITY,
        reasons,
    };
    let text = std::str::from_utf8(body)
        .map_err(|_| refused(vec!["the series is not UTF-8 text".into()]))?;
    let series = read_series(text).map_err(refused)?;
    // Screening checks the title and description like an entry (design.md, Series).
    let screened = format!("{}\n\n{}", series.title, series.description);
    let now = time::now_utc();
    let _turn = state.screening_lock.lock().await;
    // It takes its unit of the allowance before it is screened, and gives it back
    // only when the server turns it away for its own reasons (design.md, Limits and
    // abuse).
    let Some(unit) = allowance::take_for_series(state, &account.id, now.date()).await? else {
        return Err(allowance::series_refusal());
    };
    let screened = queue::screen_text(state, &account.id, None, &screened, now).await?;
    if matches!(screened, Screened::OverCap | Screened::Failed(_)) {
        allowance::give_back_series(&state.pool, unit).await?;
    }
    match screened {
        Screened::Verdict(Verdict::Pass { hazard_doubt, .. }) => {
            if let Some(doubt) = hazard_doubt {
                tracing::warn!(account = %account.id, doubt, "a series passed with a hazard doubt");
            }
            Ok(series)
        }
        // A model wrote these from the series, so they go below the delimiter.
        Screened::Verdict(Verdict::Reject { reasons }) => Err(AppError::ScreenRejected(reasons)),
        Screened::Guarded => Err(refused(vec![
            "the archive's owner has put this account under review, so the server stores no series from it until the owner clears the account"
                .into(),
        ])),
        Screened::Verdict(Verdict::Review { why }) => {
            // Kept for the owner, who confirms it (guarding the account) or dismisses it.
            series::record_refusal(&state.pool, &account.id, versioning, &series, &why).await?;
            Err(refused(vec![
                "the screen held this series back, and the archive's owner will look at it; don't send it again reworded"
                    .into(),
            ]))
        }
        Screened::OverCap => Err(AppError::Refused {
            status: StatusCode::TOO_MANY_REQUESTS,
            reasons: vec![
                "the day's screening budget is spent; send it again tomorrow (UTC)".into(),
            ],
        }),
        Screened::Failed(_) => Err(AppError::Refused {
            status: StatusCode::SERVICE_UNAVAILABLE,
            reasons: vec!["screening is unavailable just now; send it again later".into()],
        }),
    }
}

/// The text block below the delimiter for one version: its title, then its
/// description.
fn version_text(version: &Version) -> String {
    format!(
        "title: {}\n\n{}\n",
        one_line(&version.title),
        version.description
    )
}

pub async fn create(
    State(state): State<AppState>,
    account: TokenAccount,
    format: Format,
    body: Bytes,
) -> Result<Response, AppError> {
    let text = accept_series(&state, &account, None, &body).await?;
    // While embargoes are paused, so are hidden series (the owner's decision,
    // 2026-11-08): a new one is public from the start.
    let public = !state.embargoes.taken();
    let (id, key) = series::create(&state.pool, &account.id, &text, public).await?;
    let url = format!("{}/series/{id}", state.public_url);
    let (status, until) = if public {
        ("public", "")
    } else {
        (
            "hidden",
            ", and its creator makes it public from the account page in a browser; until then its entries wait",
        )
    };
    let reply = format!(
        "id: {id}\nstatus: {status}\nversion: 1\nkey: {key}\nurl: {url}\njoin: {url}/join\n\
         Keep the key private. A teammate's agent joins by sending it with POST to the join url, entries cite the series as `series: {id}@1`{until}.\n\
         {DELIMITER}\ntitle: {}\n",
        one_line(&text.title)
    );
    let json = json!({
        "id": id,
        "status": status,
        "version": 1,
        "key": key,
        "url": url,
        "join": format!("{url}/join"),
        "title": text.title,
    });
    Ok(format.reply(StatusCode::CREATED, reply, json))
}

pub async fn new_version(
    State(state): State<AppState>,
    account: TokenAccount,
    Path(id): Path<String>,
    format: Format,
    body: Bytes,
) -> Result<Response, AppError> {
    let found = creator_series(&state, &id, &account.id).await?;
    let text = accept_series(&state, &account, Some(&id), &body).await?;
    let version = series::add_version(&state.pool, found.seq, &text).await?;
    let url = format!("{}/series/{id}@{version}", state.public_url);
    let reply = format!(
        "id: {id}\nversion: {version}\nurl: {url}\n{DELIMITER}\ntitle: {}\n",
        one_line(&text.title)
    );
    let json = json!({"id": id, "version": version, "url": url, "title": text.title});
    Ok(format.reply(StatusCode::CREATED, reply, json))
}

/// The series `id` if `account_id` made it. Another member is refused; anyone else
/// gets not found.
async fn creator_series(state: &AppState, id: &str, account_id: &str) -> Result<Series, AppError> {
    let found = member_series(state, id, account_id).await?;
    if found.creator != account_id {
        return Err(AppError::Forbidden(format!(
            "only the account that made series {id} can do that"
        )));
    }
    Ok(found)
}

/// `POST /series/<id>/key`: a new join key, shown once. The old key stops working;
/// members stay members.
pub async fn rotate_key(
    State(state): State<AppState>,
    account: TokenAccount,
    Path(id): Path<String>,
    format: Format,
) -> Result<Response, AppError> {
    let found = creator_series(&state, &id, &account.id).await?;
    let key = series::rotate_key(&state.pool, found.seq).await?;
    let url = format!("{}/series/{id}", state.public_url);
    let reply = format!("id: {id}\nkey: {key}\njoin: {url}/join\n");
    let json = json!({"id": id, "key": key, "join": format!("{url}/join")});
    Ok(format.reply(StatusCode::OK, reply, json))
}

/// `POST /series/<id>/members/<account>/remove`: the account can no longer cite the
/// series. Its entries stay in it. The key rotates too, so the one the removed member
/// kept doesn't let it join again; the reply shows the new key.
pub async fn remove_member(
    State(state): State<AppState>,
    account: TokenAccount,
    Path((id, member)): Path<(String, String)>,
    format: Format,
) -> Result<Response, AppError> {
    let found = creator_series(&state, &id, &account.id).await?;
    if member == account.id {
        return Err(AppError::Conflict(format!(
            "the account that made series {id} stays its member"
        )));
    }
    if !series::remove_member(&state.pool, found.seq, &member).await? {
        return Err(AppError::NotFound);
    }
    let key = series::rotate_key(&state.pool, found.seq).await?;
    let join = format!("{}/series/{id}/join", state.public_url);
    let reply = format!("id: {id}\nremoved: {member}\nkey: {key}\njoin: {join}\n");
    Ok(format.reply(
        StatusCode::OK,
        reply,
        json!({"id": id, "removed": member, "key": key, "join": join}),
    ))
}

/// `POST /series/<id>/entries/<entry>/remove`: takes an entry out of the series. It
/// keeps the version it cites, and stops waiting on the series.
pub async fn remove_entry(
    State(state): State<AppState>,
    account: TokenAccount,
    Path((id, entry)): Path<(String, String)>,
    format: Format,
) -> Result<Response, AppError> {
    let found = creator_series(&state, &id, &account.id).await?;
    if !series::remove_entry(&state.pool, found.seq, &entry).await? {
        return Err(AppError::NotFound);
    }
    jobs::place_waiting(&state).await?;
    let reply = format!("id: {id}\nremoved: {entry}\n");
    Ok(format.reply(StatusCode::OK, reply, json!({"id": id, "removed": entry})))
}

/// The series `id` if `account_id` is a member; anything else is not found.
async fn member_series(state: &AppState, id: &str, account_id: &str) -> Result<Series, AppError> {
    match series::find(&state.pool, id).await? {
        Some(found) if series::is_member(&state.pool, found.seq, account_id).await? => Ok(found),
        _ => Err(AppError::NotFound),
    }
}

pub async fn join(
    State(state): State<AppState>,
    account: TokenAccount,
    Path(id): Path<String>,
    format: Format,
    body: Bytes,
) -> Result<Response, AppError> {
    let key = std::str::from_utf8(&body).unwrap_or("").trim();
    // A wrong key and a series that doesn't exist get the same answer.
    if !series::join(&state.pool, &id, key, &account.id).await? {
        return Err(AppError::NotFound);
    }
    let url = format!("{}/series/{id}", state.public_url);
    let reply = format!("id: {id}\nstatus: member\nurl: {url}\n");
    Ok(format.reply(
        StatusCode::OK,
        reply,
        json!({"id": id, "status": "member", "url": url}),
    ))
}

/// `GET /series/<id>` shows the latest version anyone may read; `/series/<id>@<n>`
/// shows version n, or says its description is not public yet. Members read every
/// version, and a hidden series is not found for anyone else.
pub async fn read(
    State(state): State<AppState>,
    Path(path): Path<String>,
    headers: HeaderMap,
    format: Format,
) -> Result<Response, AppError> {
    let Reading {
        path,
        format,
        as_page,
    } = Reading::of(path, format, &headers);
    let (id, asked) = match path.split_once('@') {
        Some((id, version)) => {
            let version = version.parse::<i64>().map_err(|_| AppError::NotFound)?;
            (id, Some(version))
        }
        None => (path.as_str(), None),
    };
    let found = series::find(&state.pool, id)
        .await?
        .ok_or(AppError::NotFound)?;
    let viewer = optional_token_account(&headers, &state).await?;
    let member = match &viewer {
        Some(account) => series::is_member(&state.pool, found.seq, &account.id).await?,
        None => false,
    };
    let is_creator = viewer.is_some_and(|account| account.id == found.creator);
    if !found.public && !member {
        return Err(AppError::NotFound);
    }
    let shown = match asked {
        Some(number) => Some(
            series::version(&state.pool, found.seq, number)
                .await?
                .ok_or(AppError::NotFound)?,
        ),
        None if member => series::version(&state.pool, found.seq, found.latest).await?,
        None => series::latest_readable(&state.pool, found.seq).await?,
    };
    let visible = shown.filter(|version| version.readable || member);
    let entries = series::public_entries(&state.pool, found.seq).await?;
    let status = if found.public { "public" } else { "hidden" };
    let mut text = format!(
        "id: {}\nstatus: {status}\nversions: {}\nurl: {}/series/{}\n",
        found.id, found.latest, state.public_url, found.id
    );
    match (&visible, asked) {
        (Some(version), _) => text.push_str(&format!("version: {}\n", version.version)),
        (None, Some(number)) => {
            text.push_str(&format!("version: {number}\ndescription: not public\n"));
        }
        (None, None) => text.push_str("description: not public\n"),
    }
    for listed in &entries {
        text.push_str(&format!(
            "entry: {}, account {}, {}\n",
            listed.id,
            listed.account_id,
            byline(listed.display_name.as_deref())
        ));
    }
    let members = if is_creator {
        series::members(&state.pool, found.seq).await?
    } else {
        Vec::new()
    };
    for member in &members {
        text.push_str(&format!(
            "member: {}, {}\n",
            member.account_id,
            byline(member.display_name.as_deref())
        ));
    }
    if as_page {
        let base = url::Url::parse(&format!("{}/series/{}", state.public_url, found.id)).ok();
        let description = visible
            .as_ref()
            .map(|version| markdown::to_html(&version.description, base.as_ref(), &[]));
        return Ok(page(
            StatusCode::OK,
            &SeriesPage {
                id: &found.id,
                fields: fields_above(&text),
                dropped: description
                    .as_ref()
                    .is_some_and(|rendered| rendered.dropped),
                title: visible.as_ref().map(|version| one_line(&version.title)),
                description: description
                    .map(|rendered| rendered.html)
                    .unwrap_or_default(),
            },
        ));
    }
    text.push_str(&format!("{DELIMITER}\n"));
    if let Some(version) = &visible {
        text.push_str(&version_text(version));
    }
    let json = json!({
        "id": found.id,
        "status": status,
        "versions": found.latest,
        "version": visible.as_ref().map(|version| version.version).or(asked),
        "title": visible.as_ref().map(|version| &version.title),
        "description": visible.as_ref().map(|version| &version.description),
        "entries": entries.iter().map(|listed| json!({
            "id": listed.id,
            "author": {"display_name": listed.display_name, "account_id": listed.account_id},
        })).collect::<Vec<_>>(),
        "members": is_creator.then(|| members.iter().map(|member| json!({
            "account_id": member.account_id,
            "display_name": member.display_name,
        })).collect::<Vec<_>>()),
    });
    Ok(format.reply(StatusCode::OK, text, json))
}
