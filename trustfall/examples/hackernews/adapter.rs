#![allow(dead_code)]

use std::collections::HashSet;

use hn_api::{types::Item, HnClient};
use trustfall::{
    provider::{
        field_property, resolve_coercion_with, resolve_neighbors_with, resolve_property_with,
        BasicAdapter, ContextIterator, ContextOutcomeIterator, EdgeParameters, VertexIterator,
    },
    FieldValue, Schema,
};

use crate::vertex::Vertex;

lazy_static! {
    static ref CLIENT: HnClient = HnClient::init().expect("HnClient instantiated");
    static ref SCHEMA: Schema =
        Schema::parse(include_str!("hackernews.graphql")).expect("valid schema");
}

#[derive(Debug, Clone, Default)]
pub struct HackerNewsAdapter {
    /// Set of types that implement the Item interface in the schema.
    item_subtypes: HashSet<String>,
}

impl HackerNewsAdapter {
    pub fn new() -> Self {
        Self {
            item_subtypes: SCHEMA
                .subtypes("Item")
                .expect("Item type exists")
                .map(|x| x.to_owned())
                .collect(),
        }
    }

    fn front_page(&self) -> VertexIterator<'static, Vertex> {
        self.top(Some(30))
    }

    fn top(&self, max: Option<usize>) -> VertexIterator<'static, Vertex> {
        let iterator = CLIENT
            .get_top_stories()
            .unwrap()
            .into_iter()
            .take(max.unwrap_or(usize::MAX))
            .filter_map(|id| match CLIENT.get_item(id) {
                Ok(maybe_item) => maybe_item.map(|item| item.into()),
                Err(e) => {
                    eprintln!("Got an error while fetching item: {e}");
                    None
                }
            });

        Box::new(iterator)
    }

    fn latest_stories(&self, max: Option<usize>) -> VertexIterator<'static, Vertex> {
        // Unfortunately, the HN crate we're using doesn't support getting the new stories,
        // so we're doing it manually here.
        let story_ids: Vec<u32> =
            reqwest::blocking::get("https://hacker-news.firebaseio.com/v0/newstories.json")
                .unwrap()
                .json()
                .unwrap();

        let iterator = story_ids
            .into_iter()
            .take(max.unwrap_or(usize::MAX))
            .map(move |id| CLIENT.get_item(id))
            .filter_map(|res| match res {
                Ok(maybe_item) => maybe_item.map(|item| item.into()),
                Err(e) => {
                    eprintln!("Got an error while fetching item: {e}");
                    None
                }
            });

        Box::new(iterator)
    }

    fn user(&self, username: &str) -> VertexIterator<'static, Vertex> {
        match CLIENT.get_user(username) {
            Ok(Some(user)) => {
                // Found a user by that name.
                let vertex = Vertex::from(user);
                Box::new(std::iter::once(vertex))
            }
            Ok(None) => {
                // The request succeeded but did not find a user by that name.
                Box::new(std::iter::empty())
            }
            Err(e) => {
                eprintln!("Got an error while getting user profile for user {username}: {e}",);
                Box::new(std::iter::empty())
            }
        }
    }
}

macro_rules! item_property_resolver {
    ($attr:ident) => {
        |vertex| -> FieldValue {
            if let Some(s) = vertex.as_story() {
                s.$attr.clone().into()
            } else if let Some(j) = vertex.as_job() {
                j.$attr.clone().into()
            } else if let Some(c) = vertex.as_comment() {
                c.$attr.clone().into()
            } else if let Some(p) = vertex.as_poll() {
                p.$attr.clone().into()
            } else if let Some(p) = vertex.as_poll_option() {
                p.$attr.clone().into()
            } else {
                unreachable!("{:?}", vertex)
            }
        }
    };
}

impl BasicAdapter<'static> for HackerNewsAdapter {
    type Vertex = Vertex;

    fn resolve_starting_vertices(
        &mut self,
        edge_name: &str,
        parameters: &EdgeParameters,
    ) -> VertexIterator<'static, Self::Vertex> {
        match edge_name {
            "FrontPage" => self.front_page(),
            "Top" => {
                let max = parameters.get("max").map(|v| v.as_u64().unwrap() as usize);
                self.top(max)
            }
            "LatestStory" => {
                let max = parameters.get("max").map(|v| v.as_u64().unwrap() as usize);
                self.latest_stories(max)
            }
            "User" => {
                let username_value = parameters["name"].as_str().unwrap();
                self.user(username_value)
            }
            _ => unimplemented!("unexpected starting edge: {edge_name}"),
        }
    }

    fn resolve_property(
        &mut self,
        contexts: ContextIterator<'static, Self::Vertex>,
        type_name: &str,
        property_name: &str,
    ) -> ContextOutcomeIterator<'static, Self::Vertex, FieldValue> {
        match (type_name, property_name) {
            // properties on Item and its implementers
            (type_name, "id") if self.item_subtypes.contains(type_name) => {
                resolve_property_with(contexts, item_property_resolver!(id))
            }
            (type_name, "unixTime") if self.item_subtypes.contains(type_name) => {
                resolve_property_with(contexts, item_property_resolver!(time))
            }

            // properties on Job
            ("Job", "score") => resolve_property_with(contexts, field_property!(as_job, score)),
            ("Job", "title") => resolve_property_with(contexts, field_property!(as_job, title)),
            ("Job", "url") => resolve_property_with(contexts, field_property!(as_job, url)),

            // properties on Story
            ("Story", "byUsername") => {
                resolve_property_with(contexts, field_property!(as_story, by))
            }
            ("Story", "text") => resolve_property_with(contexts, field_property!(as_story, text)),
            ("Story", "commentsCount") => {
                resolve_property_with(contexts, field_property!(as_story, descendants))
            }
            ("Story", "score") => resolve_property_with(contexts, field_property!(as_story, score)),
            ("Story", "title") => resolve_property_with(contexts, field_property!(as_story, title)),
            ("Story", "url") => resolve_property_with(contexts, field_property!(as_story, url)),

            // properties on Comment
            ("Comment", "byUsername") => {
                resolve_property_with(contexts, field_property!(as_comment, by))
            }
            ("Comment", "text") => {
                resolve_property_with(contexts, field_property!(as_comment, text))
            }
            ("Comment", "childCount") => resolve_property_with(
                contexts,
                field_property!(as_comment, kids, {
                    kids.as_ref().map(|v| v.len() as u64).unwrap_or(0).into()
                }),
            ),

            // properties on User
            ("User", "id") => resolve_property_with(contexts, field_property!(as_user, id)),
            ("User", "karma") => resolve_property_with(contexts, field_property!(as_user, karma)),
            ("User", "about") => resolve_property_with(contexts, field_property!(as_user, about)),
            ("User", "unixCreatedAt") => {
                resolve_property_with(contexts, field_property!(as_user, created))
            }
            ("User", "delay") => resolve_property_with(contexts, field_property!(as_user, delay)),
            _ => unreachable!(),
        }
    }

    fn resolve_neighbors(
        &mut self,
        contexts: ContextIterator<'static, Self::Vertex>,
        type_name: &str,
        edge_name: &str,
        _parameters: &EdgeParameters,
    ) -> ContextOutcomeIterator<'static, Self::Vertex, VertexIterator<'static, Self::Vertex>> {
        match (type_name, edge_name) {
            ("Story", "byUser") => {
                let edge_resolver =
                    |vertex: &Self::Vertex| -> VertexIterator<'static, Self::Vertex> {
                        let story = vertex.as_story().unwrap();
                        let author = story.by.as_str();
                        match CLIENT.get_user(author) {
                            Ok(None) => Box::new(std::iter::empty()), // no known author
                            Ok(Some(user)) => Box::new(std::iter::once(user.into())),
                            Err(e) => {
                                eprintln!(
                                    "API error while fetching story {} author \"{}\": {}",
                                    story.id, author, e
                                );
                                Box::new(std::iter::empty())
                            }
                        }
                    };
                resolve_neighbors_with(contexts, edge_resolver)
            }
            ("Story", "comment") => {
                let edge_resolver = |vertex: &Self::Vertex| {
                    let story = vertex.as_story().unwrap();
                    let comment_ids = story.kids.clone().unwrap_or_default();
                    let story_id = story.id;

                    let neighbors: VertexIterator<'static, Self::Vertex> =
                        Box::new(comment_ids.into_iter().filter_map(move |comment_id| {
                            match CLIENT.get_item(comment_id) {
                                Ok(None) => None,
                                Ok(Some(item)) => {
                                    if let Item::Comment(comment) = item {
                                        Some(comment.into())
                                    } else {
                                        unreachable!()
                                    }
                                }
                                Err(e) => {
                                    eprintln!(
                                        "API error while fetching story {story_id} comment {comment_id}: {e}",
                                    );
                                    None
                                }
                            }
                        }));

                    neighbors
                };
                resolve_neighbors_with(contexts, edge_resolver)
            }
            ("Comment", "byUser") => {
                let edge_resolver = |vertex: &Self::Vertex| {
                    let comment = vertex.as_comment().unwrap();
                    let author = comment.by.as_str();
                    let neighbors: VertexIterator<'static, Self::Vertex> =
                        match CLIENT.get_user(author) {
                            Ok(None) => Box::new(std::iter::empty()), // no known author
                            Ok(Some(user)) => Box::new(std::iter::once(user.into())),
                            Err(e) => {
                                eprintln!(
                                    "API error while fetching comment {} author \"{}\": {}",
                                    comment.id, author, e
                                );
                                Box::new(std::iter::empty())
                            }
                        };
                    neighbors
                };
                resolve_neighbors_with(contexts, edge_resolver)
            }
            ("Comment", "parent") => {
                let edge_resolver = |vertex: &Self::Vertex| {
                    let comment = vertex.as_comment().unwrap();
                    let comment_id = comment.id;
                    let parent_id = comment.parent;

                    let neighbors: VertexIterator<'static, Self::Vertex> = match CLIENT
                        .get_item(parent_id)
                    {
                        Ok(None) => Box::new(std::iter::empty()),
                        Ok(Some(item)) => Box::new(std::iter::once(item.into())),
                        Err(e) => {
                            eprintln!(
                                "API error while fetching comment {comment_id} parent {parent_id}: {e}",
                            );
                            Box::new(std::iter::empty())
                        }
                    };
                    neighbors
                };
                resolve_neighbors_with(contexts, edge_resolver)
            }
            ("Comment", "reply") => {
                let edge_resolver = |vertex: &Self::Vertex| {
                    let comment = vertex.as_comment().unwrap();
                    let comment_id = comment.id;
                    let reply_ids = comment.kids.clone().unwrap_or_default();

                    let neighbors: VertexIterator<'static, Self::Vertex> = Box::new(reply_ids.into_iter().filter_map(move |reply_id| {
                        match CLIENT.get_item(reply_id) {
                            Ok(None) => None,
                            Ok(Some(item)) => {
                                if let Item::Comment(c) = item {
                                    Some(c.into())
                                } else {
                                    unreachable!()
                                }
                            }
                            Err(e) => {
                                eprintln!(
                                    "API error while fetching comment {comment_id} reply {reply_id}: {e}",
                                );
                                None
                            }
                        }
                    }));
                    neighbors
                };
                resolve_neighbors_with(contexts, edge_resolver)
            }
            ("User", "submitted") => {
                let edge_resolver = |vertex: &Self::Vertex| {
                    let user = vertex.as_user().unwrap();
                    let submitted_ids = user.submitted.clone();

                    let neighbors: VertexIterator<'static, Self::Vertex> =
                        Box::new(submitted_ids.into_iter().filter_map(move |submission_id| {
                            match CLIENT.get_item(submission_id) {
                                Ok(None) => None,
                                Ok(Some(item)) => Some(item.into()),
                                Err(e) => {
                                    eprintln!(
                                    "API error while fetching submitted item {submission_id}: {e}",
                                );
                                    None
                                }
                            }
                        }));
                    neighbors
                };
                resolve_neighbors_with(contexts, edge_resolver)
            }
            _ => unreachable!("{} {}", type_name, edge_name),
        }
    }

    fn resolve_coercion(
        &mut self,
        contexts: ContextIterator<'static, Self::Vertex>,
        type_name: &str,
        coerce_to_type: &str,
    ) -> ContextOutcomeIterator<'static, Self::Vertex, bool> {
        match (type_name, coerce_to_type) {
            ("Item", "Job") => resolve_coercion_with(contexts, |v| v.as_job().is_some()),
            ("Item", "Story") => resolve_coercion_with(contexts, |v| v.as_story().is_some()),
            ("Item", "Comment") => resolve_coercion_with(contexts, |v| v.as_comment().is_some()),
            ("Item", "Poll") => resolve_coercion_with(contexts, |v| v.as_poll().is_some()),
            ("Item", "PollOption") => {
                resolve_coercion_with(contexts, |v| v.as_poll_option().is_some())
            }
            _ => unreachable!(),
        }
    }
}
