//! HTML resources that help users troubleshoot problems.
//! 
//! This provides a single endpoint, as well as necessary application data to
//! power it. The endpoint can be mounted on an actix_web App using the exposed
//! `make_service(…)` function.
//! 
//! # Examples
//! 
//! ```
//! let service = view::make_service()?;
//! let app_factory = move ||
//!     App::new()
//!         .configure(|cfg| service(cfg));
//! 
//! HttpServer::new(app_factory)
//!     .bind(port)?
//!     .run()
//!     .await?;
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

use actix_web::{get, error, web};
use maud::{DOCTYPE, html, Markup, PreEscaped};
use rand::thread_rng;
use rand::seq::SliceRandom;
use serde::{Serialize, Deserialize};
use serde_urlencoded::ser;
use std::iter;
use thiserror::Error;
use url;

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
    fn index_url(&self, query: ItemQuery) -> Result<url::Url, UrlError>;
}

impl Urls for web::HttpRequest {
    fn index_url(&self, query: ItemQuery) -> Result<url::Url, UrlError> {
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

impl From<&usize> for ItemQuery {
    fn from(idx: &usize) -> Self {
        ItemQuery {
            item: Some(*idx),
        }
    }
}

fn index_view(req: impl Urls, idx: &usize, thing: &String) -> Result<Markup, UrlError> {
    Ok(html! {
        (DOCTYPE)
        html {
            head {
                title { "Have you checked " (thing) "?" }
                style {
                    (PreEscaped("
                    body {
                        background: #dddde7;
                        font-color: #888;
                        font-family: Helvetica, sans-serif;
                        display: flex;
                        flex-direction: column;
                        justify-content: center;
                        height: 100vh;
                        margin: 0;
                    }

                    section {
                        width: 600px;
                        margin: auto;
                    }

                    p {
                        font-size: 24px;
                    }

                    a {
                        text-decoration: none;
                    }
                    "))
                }
                meta property="og:type" content="website";
                meta property="og:title" content="Troubleshooting suggestion";
                meta property="og:description" content={ "Have you checked " (thing) "?" };
            }
            body {
                section {
                    p { "Have you checked " (PreEscaped(thing)) "?" }
                    p {
                        a href=( req.index_url(ItemQuery::default())? ) { "That wasn't it, suggest something else." }
                    }
                    p {
                        a href=( req.index_url(ItemQuery::from(idx))? ) { "Share this troubleshooting suggestion." }
                    }
                }
                a href="https://github.com/ojacobson/things-to-check" {
                    img
                        style="position: absolute; top: 0; right: 0; border: 0;"
                        src="https://camo.githubusercontent.com/38ef81f8aca64bb9a64448d0d70f1308ef5341ab/68747470733a2f2f73332e616d617a6f6e6177732e636f6d2f6769746875622f726962626f6e732f666f726b6d655f72696768745f6461726b626c75655f3132313632312e706e67" alt="Fork me on GitHub"
                        data-canonical-src="https://s3.amazonaws.com/github/ribbons/forkme_right_darkblue_121621.png";
                }
            }
        }
    })
}

#[get("/")]
async fn index(
    req: web::HttpRequest,
    data: web::Data<Things>,
    query: web::Query<ItemQuery>,
) -> error::Result<Markup> {
    let thing = match query.item {
        Some(index) => data.0.get(index),
        None => data.0.choose(&mut thread_rng()),
    };

    let (index, thing) = match thing {
        Some(x) => x,
        None => return Err(error::ErrorNotFound("Not found")),
    };

    Ok(index_view(req, index, thing)?)
}

const THINGS: &str = include_str!("things-to-check.yml");

#[derive(Clone)]
struct Things(Vec<(usize, String)>);

/// Errors that can arise initializing the service.
#[derive(Error, Debug)]
pub enum Error {
    /// Indicates that the included YAML was invalid in some way. This is only
    /// fixable by recompiling the program with correct YAML.
    #[error("Unable to load Things To Check YAML: {0}")]
    DeserializeError(#[from] serde_yaml::Error)
}

/// Set up an instance of this service.
/// 
/// The returned function will configure any actix-web App with the necessary
/// state to tell people how to troubleshoot problems.
pub fn make_service() -> Result<impl Fn(&mut web::ServiceConfig) + Clone, Error> {
    let things: Vec<String> = serde_yaml::from_str(THINGS)?;
    let things = things.into_iter()
        .enumerate()
        .collect();
    let things = Things(things);

    Ok(move |cfg: &mut web::ServiceConfig| {
        cfg.data(things.clone())
            .service(index);
    })
}