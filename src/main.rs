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
use structopt::StructOpt;
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

#[derive(StructOpt)]
struct Options {
    /// The path to the root of the documentation to be checked
    #[structopt(short, long, default_value = ".")]
    root: PathBuf,

    /// Skip validation of HTTP(S) URLs
    #[structopt(short, long)]
    skip_http: bool,

    /// Path(s) to exclude, relative to the root
    #[structopt(short, long)]
    exclude: Vec<PathBuf>,

    /// Allow absolute path to join with root and evaluate
    #[structopt(short, long)]
    allow_absolute_paths: bool,
}

fn main() {
    let options = Options::from_args();

    let mut links = Vec::new();
    let mut found_error = false;

    'entries: for entry in WalkDir::new(&options.root).into_iter().filter_map(|entry| {
        let entry = entry.unwrap_or_else(|error| {
            fail!("Failed to walk directory: {}", error);
        });
        if entry.file_type().is_file() && entry.path().extension() == Some(OsStr::new("md")) {
            Some(entry)
        } else {
            None
        }
    }) {
        for exclude in &options.exclude {
            if entry.path().starts_with(options.root.join(exclude)) {
                continue 'entries;
            }
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
            Ok(_) if options.skip_http => {}
            Ok(mut url) => {
                url.set_fragment(None);
                urls.entry(url).or_insert_with(Vec::new).push(link)
            }
            Err(ParseError::RelativeUrlWithoutBase) => {
                if let Err(error) = check_path(
                    &options.root,
                    &link.target,
                    &link.file,
                    options.allow_absolute_paths,
                ) {
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
    let agent = UserAgent(format!("marker/{}", structopt::clap::crate_version!()));

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

fn check_path(
    root: &PathBuf,
    target: &str,
    file: &Path,
    allow_absolute_paths: bool,
) -> Result<(), LinkError> {
    let path = Path::new(OsStr::new(target.split('#').next().expect("string")));

    if path.is_absolute() {
        if !allow_absolute_paths {
            return Err(LinkError::PathAbsolute);
        }

        let mut path_comps = path.components();
        path_comps.next();

        if root.join(path_comps.as_path()).exists() {
            return Ok(());
        } else {
            return Err(LinkError::PathNonExistant);
        }
    }

    if !file
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
