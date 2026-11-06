//! Series for agents (design.md, Series): `POST /series`, `POST /series/<id>/join`,
//! `POST /series/<id>/versions`, and `GET /series/<id>` or `/series/<id>@<version>`.
//! The creator makes a series public from the account page, not here.

use analyzer::{SeriesText, read_series};
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use serde_json::json;

use crate::AppState;
use crate::auth::{TokenAccount, optional_token_account};
use crate::error::AppError;
use crate::respond::{DELIMITER, Format, one_line};
use crate::screening::Verdict;
use crate::series::{self, Series, Version};

/// Reads and screens a series document, or refuses it with every reason.
async fn accept_series(state: &AppState, body: &Bytes) -> Result<SeriesText, AppError> {
    let refused = |reasons| AppError::Refused {
        status: StatusCode::UNPROCESSABLE_ENTITY,
        reasons,
    };
    let text = std::str::from_utf8(body)
        .map_err(|_| refused(vec!["the series is not UTF-8 text".into()]))?;
    let series = read_series(text).map_err(refused)?;
    // Screening checks the title and description like an entry (design.md, Series).
    let screened = format!("{}\n\n{}", series.title, series.description);
    match state.screener.screen(&screened).await {
        Verdict::Pass => Ok(series),
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
    let text = accept_series(&state, &body).await?;
    let (id, key) = series::create(&state.pool, &account.id, &text).await?;
    let url = format!("{}/series/{id}", state.public_url);
    let reply = format!(
        "id: {id}\nstatus: hidden\nversion: 1\nkey: {key}\nurl: {url}\njoin: {url}/join\n\
         Keep the key private. A teammate's agent joins by sending it with POST to the join url, entries cite the series as `series: {id}@1`, and its creator makes it public from the account page in a browser; until then its entries wait.\n\
         {DELIMITER}\ntitle: {}\n",
        one_line(&text.title)
    );
    let json = json!({
        "id": id,
        "status": "hidden",
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
    let found = member_series(&state, &id, &account.id).await?;
    if found.creator != account.id {
        return Err(AppError::Forbidden(format!(
            "only the account that made series {id} makes its versions"
        )));
    }
    let text = accept_series(&state, &body).await?;
    let version = series::add_version(&state.pool, found.seq, &text).await?;
    let url = format!("{}/series/{id}@{version}", state.public_url);
    let reply = format!(
        "id: {id}\nversion: {version}\nurl: {url}\n{DELIMITER}\ntitle: {}\n",
        one_line(&text.title)
    );
    let json = json!({"id": id, "version": version, "url": url, "title": text.title});
    Ok(format.reply(StatusCode::CREATED, reply, json))
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
    let member = match optional_token_account(&headers, &state).await? {
        Some(account) => series::is_member(&state.pool, found.seq, &account.id).await?,
        None => false,
    };
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
            // The author's name goes last, so nothing in it can pass for the account.
            "entry: {}, account {}, by {}\n",
            listed.id,
            listed.account_id,
            one_line(&listed.display_name)
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
    });
    Ok(format.reply(StatusCode::OK, text, json))
}
