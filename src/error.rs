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

use hyper;
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use url::ParseError;

#[derive(Clone)]
pub enum LinkError {
    PathAbsolute,
    PathNonExistant,
    HttpStatus(hyper::status::StatusCode),
    HttpError(Arc<hyper::error::Error>),
    UrlMalformed(ParseError),
    ReferenceBroken,
}

pub enum DocumentError {
    Link {
        text: String,
        target: String,
        error: LinkError,
    },
}

pub struct DocumentLocation {
    pub path: PathBuf,
    pub line: usize,
}

impl fmt::Display for DocumentLocation {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}:{}", self.path.display(), self.line)
    }
}

pub struct LocatedDocumentError {
    pub location: DocumentLocation,
    pub error: DocumentError,
}

impl fmt::Display for LocatedDocumentError {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        match self.error {
            DocumentError::Link {
                ref text,
                ref target,
                ref error,
            } => {
                let (title, detail): (&str, Option<&dyn fmt::Display>) = match *error {
                    LinkError::PathAbsolute => ("Found absolute path", None),
                    LinkError::PathNonExistant => ("Found broken path", None),
                    LinkError::HttpStatus(ref status) => ("Found broken url", Some(status)),
                    LinkError::HttpError(ref err) => ("HTTP failure", Some(err)),
                    LinkError::UrlMalformed(ref err) => ("Found malformed URL", Some(err)),
                    LinkError::ReferenceBroken => ("Found broken reference", None),
                };
                match detail {
                    Some(detail) => write!(
                        formatter,
                        "{:22} ({} -> {}) at {} : {}",
                        title, text, target, self.location, detail
                    ),
                    None => write!(
                        formatter,
                        "{:22} ({} -> {}) at {}",
                        title, text, target, self.location
                    ),
                }
            }
        }
    }
}
