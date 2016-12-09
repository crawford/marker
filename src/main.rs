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

#[macro_use]
extern crate clap;
extern crate hyper;
extern crate pulldown_cmark;
extern crate url;
extern crate walkdir;

mod document;
mod error;

use clap::{Arg, App};
use document::{Document, Error, Event};
use error::{DocumentError, DocumentLocation, LinkError, LocatedDocumentError};
use hyper::client::Client;
use hyper::header::UserAgent;
use hyper::status::StatusCode;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::exit;
use std::rc::Rc;
use url::{ParseError, Url};
use walkdir::WalkDir;

macro_rules! fail {
    ($($print:expr),+) => {
        {
            let _ = writeln!(&mut std::io::stderr(), $($print),+);
            exit(2);
        }
    };
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
                error: error,
            },
        }
    }
}

fn main() {
    let matches = App::new("Marker")
        .version(crate_version!())
        .arg(Arg::with_name("root")
            .short("r")
            .long("root")
            .help("The path to the root of the documentation to be checked")
            .takes_value(true))
        .arg(Arg::with_name("skip-http")
            .long("skip-http")
            .help("Skip validation of HTTP[S] URLs"))
        .get_matches();

    let skip_http = matches.is_present("skip-http");
    let root = Path::new(matches.value_of("root").unwrap_or("."));
    let mut links = Vec::new();
    let mut errors = Vec::new();

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
        let contents = {
            let mut file = File::open(entry.path()).unwrap_or_else(|error| {
                fail!("Failed to open file ({}): {}",
                      entry.path().display(),
                      error);
            });
            let mut text = String::new();
            if let Err(error) = file.read_to_string(&mut text) {
                fail!("Failed to read file ({}): {}",
                      entry.path().display(),
                      error);
            };
            text
        };

        for event in Document::new(&contents) {
            match event.event {
                Event::Link { target, text } => {
                    links.push(LinkContext {
                        target: target,
                        text: text,
                        line: event.line,
                        file: entry.path().to_path_buf(),
                    })
                }
                Event::Error(Error::ReferenceBroken { target, text }) => {
                    errors.push(LinkContext {
                            target: target,
                            text: text,
                            line: event.line,
                            file: entry.path().to_path_buf(),
                        }
                        .new_error(LinkError::ReferenceBroken))
                }
            }
        }
    }

    let mut urls = HashMap::new();
    for link in links {
        match Url::parse(&link.target) {
            Ok(_) if skip_http => {}
            Ok(mut url) => {
                url.set_fragment(None);
                urls.entry(url).or_insert(Vec::new()).push(link)
            }
            Err(ParseError::RelativeUrlWithoutBase) => {
                if let Err(error) = check_path(&link.target, &link.file) {
                    errors.push(link.new_error(error))
                }
            }
            Err(error) => errors.push(link.new_error(LinkError::UrlMalformed(error))),
        }
    }

    for (url, links) in urls {
        if let Err(error) = check_url(url) {
            for link in links {
                errors.push(link.new_error(error.clone()))
            }
        }
    }

    for error in &errors {
        println!("{}", error);
    }

    if errors.len() > 0 {
        exit(1)
    }
}

fn check_url(url: Url) -> Result<(), LinkError> {
    if url.scheme() != "http" && url.scheme() != "https" {
        return Ok(());
    }

    let client = Client::new();
    let agent = UserAgent(format!("marker/{}", crate_version!()));

    let res = client.head(url.clone()).header(agent.clone()).send().and_then(|resp| {
        if resp.status == StatusCode::MethodNotAllowed {
            client.get(url.clone()).header(agent.clone()).send()
        } else {
            Ok(resp)
        }
    });
    match res {
        Ok(resp) => {
            match resp.status {
                StatusCode::Ok => Ok(()),
                status => Err(LinkError::HttpStatus(status)),
            }
        }
        Err(error) => return Err(LinkError::HttpError(Rc::new(error))),
    }
}

fn check_path(target: &str, file: &Path) -> Result<(), LinkError> {
    let path = Path::new(OsStr::new(target.split('#').next().expect("string")));
    if path.is_absolute() {
        Err(LinkError::PathAbsolute)
    } else if !file.parent().expect("non-root file path").join(path).exists() {
        Err(LinkError::PathNonExistant)
    } else {
        Ok(())
    }
}
