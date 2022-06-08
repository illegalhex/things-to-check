//! HTML resources that help users troubleshoot problems.
//!
//! This provides endpoints with helpful troubleshooting advice, as well as
//! necessary application data to power them. The endpoints can be mounted on an
//! actix_web App using the exposed `make_service(…)` function.
//!
//! # Examples
//!
//! ```
//! # use things_to_check::view;
//! # #[actix_web::main]
//! # async fn main() -> std::result::Result<(), things_to_check::view::Error> {
//! use actix_web::{App, HttpServer};
//!
//! let service = view::make_service()?;
//! let app_factory = move ||
//!     App::new()
//!         .configure(|cfg| service(cfg));
//!
//! HttpServer::new(app_factory);
//! # Ok(())
//! # }
//! ```
//!
//! # Endpoints
//!
//! * `/` (`GET`): an HTML page suggesting one thing to check.
//!
//!   Takes an optional `item` URL parameter, which must be an integer between 0
//!   and the number of options available (not provided). If `item` is provided,
//!   this endpoint returns a fixed result (the `item`th suggestion in the
//!   backing data); otherwise, it returns a randomly-selected result, for
//!   fortuitous suggesting.
//!
//!   The returned page is always `text/html` on success. Invalid `item` indices
//!   will return an error.
//!
//! * `/slack/troubleshoot` (`POST`): a Slack slash command endpoint suggesting
//!   one thing to check.
//!
//!   For information on the protocol, see [Slack's own
//!   documentation](https://api.slack.com/interactivity/slash-commands). This
//!   endpoint cheats furiously, and ignores Slack's recommendations around
//!   validating requests, as there is no sensitive information returned from or
//!   stored by this service.
//!
//!   This returns a JSON message object in a Slack-compatible format, which
//!   will print the suggestion to the channel where the `/troubleshoot` command
//!   is invoked.
//!
//! # Data
//!
//! This module creates a data item in the configured application, consisting of
//! a list of strings loaded from a YAML constant. The data comes from a file in
//! this module parsed at compile time — our target deployment environments
//! don't support modifying it without triggering a rebuild anyways. It's parsed
//! on startup, however, and invalid data can cause `make_service` to fail.
//!
//! When adding suggestions, add them at the end. This will ensure that existing
//! links to existing items are not invalidated or changed - the `item`
//! parameter to the `/` endpoint is a literal index into this list.

use actix_web::{error, get, post, web, HttpRequest, HttpResponse, Responder};
use askama::Template;
use pulldown_cmark::{html, Options, Parser};
use rand::seq::SliceRandom;
use rand::thread_rng;
use serde::{Deserialize, Serialize};
use serde_urlencoded::ser;
use std::iter;
use thiserror::Error;

#[derive(Error, Debug)]
enum UrlError {
    #[error("Unable to generate URL: {0}")]
    UrlGenerationError(error::UrlGenerationError),
    #[error("Unable to generate URL: {0}")]
    SerializationError(#[from] ser::Error),
}

// In actix-web-2.0.0, UrlGenerationError neither implements Error nor Fail,
// so thiserror can't automatically generate a From implementation for us.
// This isn't perfect, but it gets the thing shipped. This omission is fixed in
// actix_web 3.0.0, which is in alpha as of this writing.
impl From<error::UrlGenerationError> for UrlError {
    fn from(err: error::UrlGenerationError) -> Self {
        UrlError::UrlGenerationError(err)
    }
}

impl From<UrlError> for error::Error {
    fn from(err: UrlError) -> Self {
        error::ErrorInternalServerError(err)
    }
}

trait Urls {
    fn index(&self, query: &ItemQuery) -> Result<url::Url, UrlError>;

    // Askama always passes parameters from templates to functions as borrows,
    // regardless of type; receiving a reference to a usize is silly, but as a
    // result, necessary.
    fn suggestion(&self, idx: &usize) -> Result<url::Url, UrlError> {
        self.index(&ItemQuery::from(*idx))
    }

    fn new_suggestion(&self) -> Result<url::Url, UrlError> {
        self.index(&ItemQuery::default())
    }
}

impl Urls for HttpRequest {
    fn index(&self, query: &ItemQuery) -> Result<url::Url, UrlError> {
        let mut url = self.url_for("index", iter::empty::<&str>())?;

        let query = serde_urlencoded::to_string(query)?;
        url.set_query(Some(&query));

        Ok(url)
    }
}

#[derive(Serialize, Deserialize, Default)]
struct ItemQuery {
    item: Option<usize>,
}

impl From<usize> for ItemQuery {
    fn from(idx: usize) -> Self {
        ItemQuery { item: Some(idx) }
    }
}

#[derive(Template)]
#[template(path = "index.html")]
struct Suggestion {
    thing: Thing,
    req: HttpRequest,
    index: usize,
}

#[get("/")]
async fn index(
    req: HttpRequest,
    data: web::Data<Things>,
    query: web::Query<ItemQuery>,
) -> error::Result<impl Responder> {
    let thing = match query.item {
        Some(index) => data.0.get(index),
        None => data.0.choose(&mut thread_rng()),
    };

    let thing = thing.ok_or_else(|| error::ErrorNotFound("Not found"))?;
    let (index, thing) = thing.to_owned();

    let response = Suggestion { thing, req, index };
    let response = response
        .customize()
        .insert_header(("Cache-Control", "no-store"));

    Ok(response)
}

#[derive(Serialize)]
struct SlackMessage<'a> {
    response_type: &'static str,
    text: &'a String,
}

#[post("/slack/troubleshoot")]
async fn slack_troubleshoot(data: web::Data<Things>) -> error::Result<impl Responder> {
    let thing = data.0.choose(&mut thread_rng());

    let (_, thing) = thing.ok_or_else(|| error::ErrorNotFound("Not found"))?;

    let response = SlackMessage {
        response_type: "in_channel",
        text: &thing.markdown,
    };

    Ok(HttpResponse::Ok().json(response))
}

const THINGS: &str = include_str!("things-to-check.yml");

#[derive(Clone)]
struct Thing {
    markdown: String,
    html: String,
}

impl From<String> for Thing {
    fn from(markdown: String) -> Self {
        let options = Options::empty();
        let parser = Parser::new_ext(&markdown, options);

        let mut html = String::new();
        html::push_html(&mut html, parser);

        Thing { markdown, html }
    }
}

#[derive(Clone)]
struct Things(Vec<(usize, Thing)>);

fn load_things(src: &str) -> serde_yaml::Result<Things> {
    let raw_things: Vec<String> = serde_yaml::from_str(src)?;

    Ok(Things(
        raw_things
            .into_iter()
            .map(Thing::from)
            .enumerate()
            .collect(),
    ))
}

/// Errors that can arise initializing the service.
#[derive(Error, Debug)]
pub enum Error {
    /// Indicates that the included YAML was invalid in some way. This is only
    /// fixable by recompiling the program with correct YAML.
    #[error("Unable to load Things To Check YAML: {0}")]
    DeserializeError(#[from] serde_yaml::Error),
}

/// Set up an instance of this service.
///
/// The returned function will configure any actix-web App with the necessary
/// state to tell people how to troubleshoot problems.
pub fn make_service() -> Result<impl Fn(&mut web::ServiceConfig) + Clone, Error> {
    let things = load_things(THINGS)?;

    Ok(move |cfg: &mut web::ServiceConfig| {
        cfg.app_data(web::Data::new(things.clone()))
            .service(index)
            .service(slack_troubleshoot);
    })
}
