// Copyright 2016 Alex Crawford
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

mod document;
mod error;

use clap::{App, Arg};
use document::{Document, Error, Event};
use error::{DocumentError, DocumentLocation, LinkError, LocatedDocumentError};
use hyper::client::Client;
use hyper::header::UserAgent;
use hyper::net::HttpsConnector;
use hyper::status::StatusCode;
use hyper_rustls::TlsClient;
use rayon::prelude::*;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::exit;
use std::sync::Arc;
use std::time::Duration;
use url::{ParseError, Url};
use walkdir::WalkDir;

macro_rules! fail {
    ($($print:expr),+) => {
        {
            eprintln!($($print),+);
            exit(1);
        }
    };
}

macro_rules! printerror {
    ($error:expr, $flag:expr) => {{
        println!("{}", $error);
        $flag = true;
    }};
}

struct LinkContext {
    target: String,
    text: String,
    line: usize,
    file: PathBuf,
}

impl LinkContext {
    fn new_error(self, error: LinkError) -> LocatedDocumentError {
        LocatedDocumentError {
            location: DocumentLocation {
                path: self.file,
                line: self.line,
            },
            error: DocumentError::Link {
                text: self.text,
                target: self.target,
                error,
            },
        }
    }
}

fn main() {
    let matches = App::new("Marker")
        .version(clap::crate_version!())
        .arg(
            Arg::with_name("root")
                .short("r")
                .long("root")
                .help("The path to the root of the documentation to be checked")
                .takes_value(true)
                .default_value("."),
        )
        .arg(
            Arg::with_name("skip-http")
                .long("skip-http")
                .help("Skip validation of HTTP[S] URLs"),
        )
        .arg(
            Arg::with_name("exclude")
                .long("exclude")
                .short("e")
                .help("Path to exclude")
                .takes_value(true)
                .multiple(true)
                .default_value(""),
        )
        .get_matches();

    let skip_http = matches.is_present("skip-http");
    let root = Path::new(matches.value_of("root").expect("default root"));
    let excludes: Vec<_> = matches
        .values_of("exclude")
        .expect("exclude paths")
        .collect();
    let mut links = Vec::new();
    let mut found_error = false;

    for entry in WalkDir::new(root).into_iter().filter_map(|entry| {
        let entry = entry.unwrap_or_else(|error| {
            fail!("Failed to walk directory: {}", error);
        });
        if entry.file_type().is_file() && entry.path().extension() == Some(OsStr::new("md")) {
            Some(entry)
        } else {
            None
        }
    }) {
        let mut skip = false;
        for exclude in &excludes {
            let exclude_path = root.join(Path::new(exclude));
            if exclude != &"" && entry.path().starts_with(exclude_path) {
                skip = true;
                break;
            }
        }
        if skip {
            continue;
        }
        let contents = {
            let mut file = File::open(entry.path()).unwrap_or_else(|error| {
                fail!(
                    "Failed to open file ({}): {}",
                    entry.path().display(),
                    error
                );
            });
            let mut text = String::new();
            if let Err(error) = file.read_to_string(&mut text) {
                fail!(
                    "Failed to read file ({}): {}",
                    entry.path().display(),
                    error
                );
            };
            text
        };

        for event in Document::new(&contents) {
            match event.event {
                Event::Link { target, text } => links.push(LinkContext {
                    target,
                    text,
                    line: event.line,
                    file: entry.path().to_path_buf(),
                }),
                Event::Error(Error::ReferenceBroken { target, text }) => printerror!(
                    LinkContext {
                        target,
                        text,
                        line: event.line,
                        file: entry.path().to_path_buf(),
                    }
                    .new_error(LinkError::ReferenceBroken),
                    found_error
                ),
            }
        }
    }

    let mut urls = HashMap::new();
    for link in links {
        match Url::parse(&link.target) {
            Ok(_) if skip_http => {}
            Ok(mut url) => {
                url.set_fragment(None);
                urls.entry(url).or_insert_with(Vec::new).push(link)
            }
            Err(ParseError::RelativeUrlWithoutBase) => {
                if let Err(error) = check_path(&link.target, &link.file) {
                    printerror!(link.new_error(error), found_error)
                }
            }
            Err(error) => printerror!(link.new_error(LinkError::UrlMalformed(error)), found_error),
        }
    }

    for (result, links) in urls
        .into_par_iter()
        .map(|(url, links)| (check_url(&url), links))
        .collect::<Vec<_>>()
    {
        if let Err(error) = result {
            for link in links {
                printerror!(link.new_error(error.clone()), found_error)
            }
        }
    }

    if found_error {
        exit(1)
    }
}

fn check_url(url: &Url) -> Result<(), LinkError> {
    if url.scheme() != "http" && url.scheme() != "https" {
        return Ok(());
    }

    let mut client = Client::with_connector(HttpsConnector::new(TlsClient::new()));
    client.set_read_timeout(Some(Duration::from_secs(10)));
    let agent = UserAgent(format!("marker/{}", clap::crate_version!()));

    let res = client
        .head(url.clone())
        .header(agent.clone())
        .send()
        .and_then(|resp| {
            if resp.status == StatusCode::MethodNotAllowed {
                client.get(url.clone()).header(agent.clone()).send()
            } else {
                Ok(resp)
            }
        });
    match res {
        Ok(resp) => match resp.status {
            StatusCode::Ok => Ok(()),
            status => Err(LinkError::HttpStatus(status)),
        },
        Err(error) => Err(LinkError::HttpError(Arc::new(error))),
    }
}

fn check_path(target: &str, file: &Path) -> Result<(), LinkError> {
    let path = Path::new(OsStr::new(target.split('#').next().expect("string")));
    if path.is_absolute() {
        Err(LinkError::PathAbsolute)
    } else if !file
        .parent()
        .expect("non-root file path")
        .join(path)
        .exists()
    {
        Err(LinkError::PathNonExistant)
    } else {
        Ok(())
    }
}
