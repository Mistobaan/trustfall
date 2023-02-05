#![allow(unused_variables, dead_code, unreachable_code)]

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::sync::Arc;

use crate::ir::{
    Argument, ContextField, FieldRef, IREdge, IRFold, IRQuery, IRVertex, Operation, Recursive,
};
use crate::{
    interpreter::basic_adapter::{ContextIterator, ContextOutcomeIterator},
    ir::{Eid, FieldValue, IRQueryComponent, Vid},
};

use super::execution::compute_context_field_with_separate_value;
use super::{Adapter, InterpretedQuery};

mod candidates;
pub use candidates::{CandidateValue, RangeBoundKind, RangeEndpoint};

pub trait VertexInfo {
    fn current_component(&self) -> &IRQueryComponent;

    fn current_vertex(&self) -> &IRVertex;

    fn query_arguments(&self) -> &BTreeMap<Arc<str>, FieldValue>;

    fn coerced_to_type(&self) -> Option<&Arc<str>> {
        let vertex = self.current_vertex();
        if vertex.coerced_from_type.is_some() {
            Some(&vertex.type_name)
        } else {
            None
        }
    }

    fn static_field_value(&self, field_name: &str) -> Option<CandidateValue<&'_ FieldValue>> {
        let vertex = self.current_vertex();

        let is_null = vertex
            .filters
            .iter()
            .any(|op| matches!(op, Operation::IsNull(..)));
        let is_not_null = vertex
            .filters
            .iter()
            .any(|op| matches!(op, Operation::IsNotNull(..)));

        if is_null && is_not_null {
            // The value can't be both null and non-null at the same time.
            return Some(CandidateValue::Impossible);
        }

        let mut candidate = if is_null {
            Some(CandidateValue::Single(&FieldValue::NULL))
        } else {
            None
        };

        let arguments = self.query_arguments();
        for filter_operation in &vertex.filters {
            match filter_operation {
                Operation::Equals(_, Argument::Variable(var)) => {
                    let value = &arguments[&var.variable_name];
                    if let Some(candidate) = candidate.as_mut() {
                        candidate.merge(CandidateValue::Single(value));
                    } else {
                        candidate = Some(CandidateValue::Single(value));
                    }
                }
                Operation::OneOf(_, Argument::Variable(var)) => {
                    let values: Vec<&FieldValue> = arguments[&var.variable_name]
                        .as_vec()
                        .expect("OneOf operation using a non-vec FieldValue")
                        .iter()
                        .map(AsRef::as_ref)
                        .collect();
                    if let Some(candidate) = candidate.as_mut() {
                        candidate.merge(CandidateValue::Multiple(values));
                    } else {
                        candidate = Some(CandidateValue::Multiple(values));
                    }
                }
                _ => {}
            }
        }

        candidate
    }

    fn static_field_range(&self, field_name: &str) -> Option<&RangeBoundKind> {
        todo!()
    }

    /// Only the first matching `@tag` value is returned.
    fn dynamic_field_value(&self, field_name: &str) -> Option<DynamicallyResolvedValue>;

    // fn dynamic_field_range(&self, field_name: &str) -> Option<DynamicallyResolvedGeneric<RangeBoundKind>>;

    // non-optional, non-recursed, non-folded edge
    // TODO: What happens if the same edge exists more than once in a given scope?
    fn first_required_edge(&self, edge_name: &str) -> Option<EdgeInfo>;

    // optional, recursed, or folded edge;
    // recursed because recursion always starts at depth 0
    // TODO: What happens if the same edge exists more than once in a given scope?
    fn first_edge(&self, edge_name: &str) -> Option<EdgeInfo>;
}

#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct QueryInfo {
    pub(crate) query: InterpretedQuery,
    pub(crate) current_vertex: Vid,
    pub(crate) crossing_eid: Option<Eid>,
}

impl QueryInfo {
    #[inline]
    pub(crate) fn new(
        query: InterpretedQuery,
        current_vertex: Vid,
        crossing_eid: Option<Eid>,
    ) -> Self {
        Self {
            query,
            current_vertex,
            crossing_eid,
        }
    }

    #[inline]
    pub(crate) fn ir_query(&self) -> &IRQuery {
        &self.query.indexed_query.ir_query
    }

    #[inline]
    pub(crate) fn arguments(&self) -> &Arc<BTreeMap<Arc<str>, FieldValue>> {
        &self.query.arguments
    }

    #[inline]
    pub fn at_vid(&self) -> Vid {
        self.current_vertex
    }

    #[inline]
    pub fn crossing_eid(&self) -> &Option<Eid> {
        &self.crossing_eid
    }

    #[inline]
    pub fn here(&self) -> LocalQueryInfo {
        LocalQueryInfo {
            query: self.clone(),
            current_vertex: self.current_vertex,
        }
    }

    #[inline]
    pub fn destination(&self) -> Option<LocalQueryInfo> {
        self.crossing_eid.map(|eid| {
            let current_vertex = match &self.query.indexed_query.eids[&eid] {
                crate::ir::indexed::EdgeKind::Regular(regular) => regular.to_vid,
                crate::ir::indexed::EdgeKind::Fold(fold) => fold.to_vid,
            };
            LocalQueryInfo {
                query: self.clone(),
                current_vertex,
            }
        })
    }
}

#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct LocalQueryInfo {
    query: QueryInfo,
    current_vertex: Vid,
}

impl LocalQueryInfo {
    fn make_non_folded_edge_info(&self, edge: &IREdge) -> EdgeInfo {
        let neighboring_info = NeighboringQueryInfo {
            query: self.query.clone(),
            starting_vertex: self.current_vertex,
            neighbor_vertex: edge.to_vid,
            neighbor_path: vec![edge.eid],
        };
        EdgeInfo {
            eid: edge.eid,
            optional: edge.optional,
            recursive: edge.recursive.clone(),
            folded: false,
            destination: neighboring_info,
        }
    }

    fn make_folded_edge_info(&self, fold: &IRFold) -> EdgeInfo {
        let neighboring_info = NeighboringQueryInfo {
            query: self.query.clone(),
            starting_vertex: self.current_vertex,
            neighbor_vertex: fold.to_vid,
            neighbor_path: vec![fold.eid],
        };
        EdgeInfo {
            eid: fold.eid,
            optional: true,
            recursive: None,
            folded: true,
            destination: neighboring_info,
        }
    }
}

impl VertexInfo for LocalQueryInfo {
    #[inline]
    fn current_vertex(&self) -> &IRVertex {
        &self.current_component().vertices[&self.current_vertex]
    }

    #[inline]
    fn current_component(&self) -> &IRQueryComponent {
        &self.query.query.indexed_query.vids[&self.current_vertex]
    }

    #[inline]
    fn query_arguments(&self) -> &BTreeMap<Arc<str>, FieldValue> {
        self.query.arguments()
    }

    fn dynamic_field_value(&self, field_name: &str) -> Option<DynamicallyResolvedValue> {
        let vertex = self.current_vertex();
        for filter_operation in &vertex.filters {
            match filter_operation {
                // TODO: handle tags of fold-specific fields
                Operation::Equals(_, Argument::Tag(FieldRef::ContextField(context_field))) => {
                    return Some(DynamicallyResolvedValue {
                        query: self.query.clone(),
                        vid: vertex.vid,
                        resolve_on_component: self.query.query.indexed_query.vids[&vertex.vid]
                            .clone(),
                        context_field: context_field.clone(),
                        is_multiple: false,
                    });
                }
                Operation::OneOf(_, Argument::Tag(FieldRef::ContextField(context_field))) => {
                    return Some(DynamicallyResolvedValue {
                        query: self.query.clone(),
                        vid: vertex.vid,
                        resolve_on_component: self.query.query.indexed_query.vids[&vertex.vid]
                            .clone(),
                        context_field: context_field.clone(),
                        is_multiple: true,
                    });
                }
                _ => {}
            }
        }

        None
    }

    // fn dynamic_field_range(&self, field_name: &str) -> Option<DynamicallyResolvedGeneric<RangeBoundKind>> {
    //     todo!()
    // }

    // non-optional, non-recursed, non-folded edge
    fn first_required_edge(&self, edge_name: &str) -> Option<EdgeInfo> {
        // TODO: What happens if the same edge exists more than once in a given scope?
        let component = self.current_component();
        let current_vertex = self.current_vertex();
        let first_matching_edge = component.edges.values().find(|edge| {
            edge.from_vid == current_vertex.vid
                && !edge.optional
                && edge.recursive.is_none()
                && edge.edge_name.as_ref() == edge_name
        });
        first_matching_edge.map(|edge| self.make_non_folded_edge_info(edge.as_ref()))
    }

    fn first_edge(&self, edge_name: &str) -> Option<EdgeInfo> {
        // TODO: What happens if the same edge exists more than once in a given scope?
        let component = self.current_component();
        let current_vertex = self.current_vertex();
        let first_matching_edge = component.edges.values().find(|edge| {
            edge.from_vid == current_vertex.vid && edge.edge_name.as_ref() == edge_name
        });
        first_matching_edge
            .map(|edge| self.make_non_folded_edge_info(edge.as_ref()))
            .or_else(|| {
                component
                    .folds
                    .values()
                    .find(|fold| {
                        fold.from_vid == current_vertex.vid && fold.edge_name.as_ref() == edge_name
                    })
                    .map(|fold| self.make_folded_edge_info(fold.as_ref()))
            })
    }
}

#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct EdgeInfo {
    eid: Eid,
    optional: bool,
    recursive: Option<Recursive>,
    folded: bool,
    destination: NeighboringQueryInfo,
}

impl EdgeInfo {
    pub fn destination(&self) -> &NeighboringQueryInfo {
        &self.destination
    }
}

#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct NeighboringQueryInfo {
    query: QueryInfo,
    starting_vertex: Vid,
    neighbor_vertex: Vid,
    neighbor_path: Vec<Eid>,
}

impl NeighboringQueryInfo {
    fn make_non_folded_edge_info(&self, edge: &IREdge) -> EdgeInfo {
        let mut neighbor_path = self.neighbor_path.clone();
        neighbor_path.push(edge.eid);
        let neighboring_info = NeighboringQueryInfo {
            query: self.query.clone(),
            starting_vertex: self.starting_vertex,
            neighbor_vertex: edge.to_vid,
            neighbor_path,
        };
        EdgeInfo {
            eid: edge.eid,
            optional: edge.optional,
            recursive: edge.recursive.clone(),
            folded: false,
            destination: neighboring_info,
        }
    }

    fn make_folded_edge_info(&self, fold: &IRFold) -> EdgeInfo {
        let mut neighbor_path = self.neighbor_path.clone();
        neighbor_path.push(fold.eid);
        let neighboring_info = NeighboringQueryInfo {
            query: self.query.clone(),
            starting_vertex: self.starting_vertex,
            neighbor_vertex: fold.to_vid,
            neighbor_path,
        };
        EdgeInfo {
            eid: fold.eid,
            optional: true,
            recursive: None,
            folded: true,
            destination: neighboring_info,
        }
    }
}

impl VertexInfo for NeighboringQueryInfo {
    #[inline]
    fn current_vertex(&self) -> &IRVertex {
        &self.current_component().vertices[&self.neighbor_vertex]
    }

    #[inline]
    fn current_component(&self) -> &IRQueryComponent {
        &self.query.query.indexed_query.vids[&self.neighbor_vertex]
    }

    #[inline]
    fn query_arguments(&self) -> &BTreeMap<Arc<str>, FieldValue> {
        self.query.arguments()
    }

    fn dynamic_field_value(&self, field_name: &str) -> Option<DynamicallyResolvedValue> {
        let vertex = self.current_vertex();

        for filter_operation in &vertex.filters {
            // Before deciding that some tag matches, we have to check if it corresponds
            // to field whose vertex has already been resolved.
            //
            // Here's an example where this is important:
            // {
            //     Foo {
            //         bar {
            //             number @tag @output
            //             baz {
            //                 target @filter(op: "=", value: ["%number"])
            //             }
            //         }
            //     }
            // }
            // Imagine execution is currently at `Foo`, and the adapter checked whether
            // the `target` property at neighbor path `-> bar -> baz` has known values.
            // Despite the use of `%number` on that property, the answer is "no" --
            // the value isn't known *yet* at the point of query evaluation of the caller.
            //
            // This is why we ensure that the tagged value came from a Vid that is at or before
            // the Vid where the caller currently stands.
            match filter_operation {
                // TODO: handle tags of fold-specific fields
                Operation::Equals(_, Argument::Tag(FieldRef::ContextField(context_field))) => {
                    if context_field.vertex_id <= self.starting_vertex {
                        return Some(DynamicallyResolvedValue {
                            query: self.query.clone(),
                            vid: vertex.vid,
                            context_field: context_field.clone(),
                            resolve_on_component: self.query.query.indexed_query.vids
                                [&self.starting_vertex]
                                .clone(),
                            is_multiple: false,
                        });
                    }
                }
                Operation::OneOf(_, Argument::Tag(FieldRef::ContextField(context_field))) => {
                    if context_field.vertex_id <= self.starting_vertex {
                        return Some(DynamicallyResolvedValue {
                            query: self.query.clone(),
                            vid: vertex.vid,
                            context_field: context_field.clone(),
                            resolve_on_component: self.query.query.indexed_query.vids
                                [&self.starting_vertex]
                                .clone(),
                            is_multiple: true,
                        });
                    }
                }
                _ => {}
            }
        }

        None
    }

    // fn dynamic_field_range(&self, field_name: &str) -> Option<DynamicallyResolvedGeneric<RangeBoundKind>> {
    //     todo!()
    // }

    fn first_required_edge(&self, edge_name: &str) -> Option<EdgeInfo> {
        // TODO: What happens if the same edge exists more than once in a given scope?
        let component = self.current_component();
        let current_vertex = self.current_vertex();
        let first_matching_edge = component.edges.values().find(|edge| {
            edge.from_vid == current_vertex.vid
                && !edge.optional
                && edge.recursive.is_none()
                && edge.edge_name.as_ref() == edge_name
        });
        first_matching_edge.map(|edge| self.make_non_folded_edge_info(edge.as_ref()))
    }

    fn first_edge(&self, edge_name: &str) -> Option<EdgeInfo> {
        // TODO: What happens if the same edge exists more than once in a given scope?
        let component = self.current_component();
        let current_vertex = self.current_vertex();
        let first_matching_edge = component.edges.values().find(|edge| {
            edge.from_vid == current_vertex.vid && edge.edge_name.as_ref() == edge_name
        });
        first_matching_edge
            .map(|edge| self.make_non_folded_edge_info(edge.as_ref()))
            .or_else(|| {
                component
                    .folds
                    .values()
                    .find(|fold| {
                        fold.from_vid == current_vertex.vid && fold.edge_name.as_ref() == edge_name
                    })
                    .map(|fold| self.make_folded_edge_info(fold.as_ref()))
            })
    }
}

#[non_exhaustive]
pub struct DynamicallyResolvedValue {
    query: QueryInfo,
    vid: Vid,
    resolve_on_component: Arc<IRQueryComponent>,
    context_field: ContextField,
    is_multiple: bool,
}

impl DynamicallyResolvedValue {
    pub fn resolve<
        'vertex,
        VertexT: Debug + Clone + 'vertex,
        AdapterT: Adapter<'vertex, DataToken = VertexT>,
    >(
        mut self,
        adapter: &mut AdapterT,
        contexts: ContextIterator<'vertex, VertexT>,
    ) -> ContextOutcomeIterator<'vertex, VertexT, CandidateValue<FieldValue>> {
        // let component = &self.query.query.indexed_query.vids[&self.vid].clone();
        let iterator = compute_context_field_with_separate_value(
            adapter,
            &mut self.query,
            &self.resolve_on_component,
            &self.context_field,
            contexts,
        );
        let context_field_vid = self.context_field.vertex_id;
        let nullable_context_field = self.context_field.field_type.nullable;
        if self.is_multiple {
            Box::new(iterator.map(move |(ctx, value)| {
                match value {
                    FieldValue::List(v) => (ctx, CandidateValue::Multiple(v)),
                    FieldValue::Null => {
                        // Either a nullable field was tagged, or
                        // the @tag is inside an @optional scope that doesn't exist.
                        let candidate = if ctx.tokens[&context_field_vid].is_none() {
                            // @optional scope that didn't exist. Our query rules say that
                            // any filters using this tag *must* pass.
                            CandidateValue::All
                        } else {
                            // The field must have been nullable.
                            debug_assert!(
                                nullable_context_field,
                                "tagged field {:?} was not nullable but received a null value for it",
                                self.context_field,
                            );
                            CandidateValue::Impossible
                        };
                        (ctx, candidate)
                    }
                    bad_value => {
                        panic!(
                            "\
tagged field named {} of type {:?} produced an invalid value: {bad_value:?}",
                            self.context_field.field_name, self.context_field.field_type,
                        )
                    }
                }
            }))
        } else {
            Box::new(iterator.map(move |(ctx, value)| match value {
                null_value @ FieldValue::Null => {
                    // Either a nullable field was tagged, or
                    // the @tag is inside an @optional scope that doesn't exist.
                    let candidate = if ctx.tokens[&context_field_vid].is_none() {
                        // @optional scope that didn't exist. Our query rules say that
                        // any filters using this tag *must* pass.
                        CandidateValue::All
                    } else {
                        // The field must have been nullable.
                        debug_assert!(
                            nullable_context_field,
                            "tagged field {:?} was not nullable but received a null value for it",
                            self.context_field,
                        );
                        CandidateValue::Single(null_value)
                    };
                    (ctx, candidate)
                }
                other_value => (ctx, CandidateValue::Single(other_value)),
            }))
        }
    }
}
