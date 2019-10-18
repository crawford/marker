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

use pulldown_cmark::{Event as ParserEvent, LinkType, OffsetIter, Options, Parser, Tag};
use std::ops::Range;

pub struct Document<'a> {
    parser: OffsetIter<'a>,
    newlines: Vec<usize>,

    code_block: bool,
    last_text: Option<String>,
}

#[derive(Debug, PartialEq)]
pub struct LocatedEvent {
    pub event: Event,
    pub line: usize,
}

#[derive(Debug, PartialEq)]
pub enum Event {
    // Link { target: &'a str, text: &'a str },
    Link { target: String, text: String },
    Error(Error),
}

#[derive(Debug, PartialEq)]
pub enum Error {
    // ReferenceBroken{ target: &'a str, text: &'a str },
    ReferenceBroken { target: String, text: String },
}

impl<'a> Document<'a> {
    pub fn new(contents: &str) -> Document {
        Document {
            parser: Parser::new_with_broken_link_callback(
                contents,
                Options::all(),
                Some(&|label, _| Some((String::new(), label.to_string()))),
            )
            .into_offset_iter(),
            newlines: contents.match_indices('\n').map(|(i, _)| i).collect(),

            code_block: false,
            last_text: None,
        }
    }

    fn new_located_event(&self, event: Event, span: Range<usize>) -> LocatedEvent {
        LocatedEvent {
            event,
            line: self
                .newlines
                .iter()
                .take_while(|&&i| i < span.start)
                .count()
                + 1,
        }
    }
}

impl<'a> Iterator for Document<'a> {
    type Item = LocatedEvent;

    fn next(&mut self) -> Option<LocatedEvent> {
        while let Some((event, position)) = self.parser.next() {
            match event {
                ParserEvent::Text(ref text) if !self.code_block => {
                    self.last_text = Some(text.to_string());
                }
                ParserEvent::End(Tag::Link(link_type, target, text)) => match link_type {
                    LinkType::Inline
                    | LinkType::Reference
                    | LinkType::Collapsed
                    | LinkType::Shortcut => {
                        return Some(self.new_located_event(
                            Event::Link {
                                target: target.to_string(),
                                text: self.last_text.clone().expect("some last text"),
                            },
                            position,
                        ))
                    }
                    LinkType::ReferenceUnknown
                    | LinkType::CollapsedUnknown
                    | LinkType::ShortcutUnknown => {
                        return Some(self.new_located_event(
                            Event::Error(Error::ReferenceBroken {
                                target: target.to_string(),
                                text: text.to_string(),
                            }),
                            position,
                        ))
                    }
                    _ => {}
                },
                ParserEvent::Start(Tag::CodeBlock(_)) => self.code_block = true,
                ParserEvent::End(Tag::CodeBlock(_)) => self.code_block = false,
                _ => {}
            }
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty() {
        let mut doc = Document::new("");
        assert_eq!(doc.next(), None);
    }

    #[test]
    fn link() {
        let mut doc = Document::new("[text](target)");
        assert_eq!(
            doc.next(),
            Some(LocatedEvent {
                event: Event::Link {
                    target: "target".to_string(),
                    text: "text".to_string(),
                },
                line: 1,
            })
        );
        assert_eq!(doc.next(), None);
    }

    #[test]
    fn links() {
        let mut doc = Document::new("[text 1](target1)\n[text 2](target2)");
        assert_eq!(
            doc.next(),
            Some(LocatedEvent {
                event: Event::Link {
                    target: "target1".to_string(),
                    text: "text 1".to_string(),
                },
                line: 1,
            })
        );
        assert_eq!(
            doc.next(),
            Some(LocatedEvent {
                event: Event::Link {
                    target: "target2".to_string(),
                    text: "text 2".to_string(),
                },
                line: 2,
            })
        );
        assert_eq!(doc.next(), None);
    }

    #[test]
    fn full_reference_links() {
        let mut doc = Document::new("[t 1][ref 1]\nText [t 2][ref 2]\n\n[ref 1]: 1\n[ref 2]: 2");
        assert_eq!(
            doc.next(),
            Some(LocatedEvent {
                event: Event::Link {
                    target: "1".to_string(),
                    text: "t 1".to_string(),
                },
                line: 1,
            })
        );
        assert_eq!(
            doc.next(),
            Some(LocatedEvent {
                event: Event::Link {
                    target: "2".to_string(),
                    text: "t 2".to_string(),
                },
                line: 2,
            })
        );
        assert_eq!(doc.next(), None);
    }

    #[test]
    fn broken_reference() {
        let mut doc = Document::new("This is a [broken reference].");
        assert_eq!(
            doc.next(),
            Some(LocatedEvent {
                event: Event::Error(Error::ReferenceBroken {
                    target: String::new(),
                    text: "broken reference".to_string()
                }),
                line: 1
            })
        );
        assert_eq!(doc.next(), None);
    }

    #[test]
    fn tasklist() {
        let mut doc = Document::new("- [ ] Item 1\n- [ ] Item 2");
        assert_eq!(doc.next(), None);
    }
}
