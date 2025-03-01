use std::{
    cell::RefCell,
    collections::{btree_map, BTreeMap, VecDeque},
    convert::TryInto,
    fmt::Debug,
    marker::PhantomData,
    rc::Rc,
    sync::Arc,
};

use serde::{Deserialize, Serialize};

use crate::ir::{indexed::IndexedQuery, EdgeParameters, FieldValue};

use super::{
    execution::interpret_ir,
    trace::{FunctionCall, Opid, Trace, TraceOp, TraceOpContent, YieldValue},
    Adapter, ContextIterator, ContextOutcomeIterator, DataContext, QueryInfo, VertexIterator,
};

#[derive(Clone, Debug)]
struct TraceReaderAdapter<'trace, Vertex>
where
    Vertex: Clone + Debug + PartialEq + Eq + Serialize + 'trace,
    for<'de2> Vertex: Deserialize<'de2>,
{
    next_op: Rc<RefCell<btree_map::Iter<'trace, Opid, TraceOp<Vertex>>>>,
}

fn advance_ref_iter<T, Iter: Iterator<Item = T>>(iter: &RefCell<Iter>) -> Option<T> {
    // We do this through a separate function to ensure the mut borrow is dropped
    // as early as possible, to avoid overlapping mut borrows.
    iter.borrow_mut().next()
}

#[derive(Debug)]
struct TraceReaderStartingVerticesIter<'trace, Vertex>
where
    Vertex: Clone + Debug + PartialEq + Eq + Serialize + 'trace,
    for<'de2> Vertex: Deserialize<'de2>,
{
    exhausted: bool,
    parent_opid: Opid,
    inner: Rc<RefCell<btree_map::Iter<'trace, Opid, TraceOp<Vertex>>>>,
}

#[allow(unused_variables)]
impl<'trace, Vertex> Iterator for TraceReaderStartingVerticesIter<'trace, Vertex>
where
    Vertex: Clone + Debug + PartialEq + Eq + Serialize + 'trace,
    for<'de2> Vertex: Deserialize<'de2>,
{
    type Item = Vertex;

    fn next(&mut self) -> Option<Self::Item> {
        assert!(!self.exhausted);

        let (_, trace_op) = advance_ref_iter(self.inner.as_ref())
            .expect("Expected to have an item but found none.");
        assert_eq!(
            self.parent_opid,
            trace_op
                .parent_opid
                .expect("Expected an operation with a parent_opid."),
            "Expected parent_opid {:?} did not match operation {:#?}",
            self.parent_opid,
            trace_op,
        );

        match &trace_op.content {
            TraceOpContent::OutputIteratorExhausted => {
                self.exhausted = true;
                None
            }
            TraceOpContent::YieldFrom(YieldValue::ResolveStartingVertices(vertex)) => {
                Some(vertex.clone())
            }
            _ => unreachable!(),
        }
    }
}

struct TraceReaderResolvePropertiesIter<'trace, Vertex>
where
    Vertex: Clone + Debug + PartialEq + Eq + Serialize + 'trace,
    for<'de2> Vertex: Deserialize<'de2>,
{
    exhausted: bool,
    parent_opid: Opid,
    contexts: ContextIterator<'trace, Vertex>,
    input_batch: VecDeque<DataContext<Vertex>>,
    inner: Rc<RefCell<btree_map::Iter<'trace, Opid, TraceOp<Vertex>>>>,
}

#[allow(unused_variables)]
impl<'trace, Vertex> Iterator for TraceReaderResolvePropertiesIter<'trace, Vertex>
where
    Vertex: Clone + Debug + PartialEq + Eq + Serialize + 'trace,
    for<'de2> Vertex: Deserialize<'de2>,
{
    type Item = (DataContext<Vertex>, FieldValue);

    fn next(&mut self) -> Option<Self::Item> {
        assert!(!self.exhausted);
        let next_op = loop {
            let (_, input_op) = advance_ref_iter(self.inner.as_ref())
                .expect("Expected to have an item but found none.");
            assert_eq!(
                self.parent_opid,
                input_op
                    .parent_opid
                    .expect("Expected an operation with a parent_opid."),
                "Expected parent_opid {:?} did not match operation {:#?}",
                self.parent_opid,
                input_op,
            );

            if let TraceOpContent::AdvanceInputIterator = &input_op.content {
                let input_data = self.contexts.next();

                let (_, input_op) = advance_ref_iter(self.inner.as_ref())
                    .expect("Expected to have an item but found none.");
                assert_eq!(
                    self.parent_opid,
                    input_op
                        .parent_opid
                        .expect("Expected an operation with a parent_opid."),
                    "Expected parent_opid {:?} did not match operation {:#?}",
                    self.parent_opid,
                    input_op,
                );

                if let TraceOpContent::YieldInto(context) = &input_op.content {
                    let input_context = input_data.unwrap();
                    assert_eq!(context, &input_context);
                    self.input_batch.push_back(input_context);
                } else if let TraceOpContent::InputIteratorExhausted = &input_op.content {
                    assert_eq!(None, input_data);
                } else {
                    unreachable!();
                }
            } else {
                break input_op;
            }
        };

        match &next_op.content {
            TraceOpContent::YieldFrom(YieldValue::ResolveProperty(trace_context, value)) => {
                let input_context = self.input_batch.pop_front().unwrap();
                assert_eq!(trace_context, &input_context);
                Some((input_context, value.clone()))
            }
            TraceOpContent::OutputIteratorExhausted => {
                assert_eq!(None, self.input_batch.pop_front());
                self.exhausted = true;
                None
            }
            _ => unreachable!(),
        }
    }
}

struct TraceReaderResolveCoercionIter<'query, 'trace, Vertex>
where
    Vertex: Clone + Debug + PartialEq + Eq + Serialize + 'query,
    for<'de2> Vertex: Deserialize<'de2>,
    'trace: 'query,
{
    exhausted: bool,
    parent_opid: Opid,
    contexts: ContextIterator<'query, Vertex>,
    input_batch: VecDeque<DataContext<Vertex>>,
    inner: Rc<RefCell<btree_map::Iter<'trace, Opid, TraceOp<Vertex>>>>,
}

#[allow(unused_variables)]
impl<'query, 'trace, Vertex> Iterator for TraceReaderResolveCoercionIter<'query, 'trace, Vertex>
where
    Vertex: Clone + Debug + PartialEq + Eq + Serialize + 'query,
    for<'de2> Vertex: Deserialize<'de2>,
    'trace: 'query,
{
    type Item = (DataContext<Vertex>, bool);

    fn next(&mut self) -> Option<Self::Item> {
        assert!(!self.exhausted);
        let next_op = loop {
            let (_, input_op) = advance_ref_iter(self.inner.as_ref())
                .expect("Expected to have an item but found none.");
            assert_eq!(
                self.parent_opid,
                input_op
                    .parent_opid
                    .expect("Expected an operation with a parent_opid."),
                "Expected parent_opid {:?} did not match operation {:#?}",
                self.parent_opid,
                input_op,
            );

            if let TraceOpContent::AdvanceInputIterator = &input_op.content {
                let input_data = self.contexts.next();

                let (_, input_op) = advance_ref_iter(self.inner.as_ref())
                    .expect("Expected to have an item but found none.");
                assert_eq!(
                    self.parent_opid,
                    input_op
                        .parent_opid
                        .expect("Expected an operation with a parent_opid."),
                    "Expected parent_opid {:?} did not match operation {:#?}",
                    self.parent_opid,
                    input_op,
                );

                if let TraceOpContent::YieldInto(context) = &input_op.content {
                    let input_context = input_data.unwrap();
                    assert_eq!(context, &input_context);

                    self.input_batch.push_back(input_context);
                } else if let TraceOpContent::InputIteratorExhausted = &input_op.content {
                    assert_eq!(None, input_data);
                } else {
                    unreachable!();
                }
            } else {
                break input_op;
            }
        };

        match &next_op.content {
            TraceOpContent::YieldFrom(YieldValue::ResolveCoercion(trace_context, can_coerce)) => {
                let input_context = self.input_batch.pop_front().unwrap();
                assert_eq!(trace_context, &input_context);
                Some((input_context, *can_coerce))
            }
            TraceOpContent::OutputIteratorExhausted => {
                assert_eq!(None, self.input_batch.pop_front());
                self.exhausted = true;
                None
            }
            _ => unreachable!(),
        }
    }
}

struct TraceReaderResolveNeighborsIter<'query, 'trace, Vertex>
where
    Vertex: Clone + Debug + PartialEq + Eq + Serialize + 'query,
    for<'de2> Vertex: Deserialize<'de2>,
    'trace: 'query,
{
    exhausted: bool,
    parent_opid: Opid,
    contexts: ContextIterator<'query, Vertex>,
    input_batch: VecDeque<DataContext<Vertex>>,
    inner: Rc<RefCell<btree_map::Iter<'trace, Opid, TraceOp<Vertex>>>>,
}

impl<'query, 'trace, Vertex> Iterator for TraceReaderResolveNeighborsIter<'query, 'trace, Vertex>
where
    Vertex: Clone + Debug + PartialEq + Eq + Serialize + 'query,
    for<'de2> Vertex: Deserialize<'de2>,
    'trace: 'query,
{
    type Item = (DataContext<Vertex>, VertexIterator<'query, Vertex>);

    fn next(&mut self) -> Option<Self::Item> {
        assert!(!self.exhausted);
        let next_op = loop {
            let (_, input_op) = advance_ref_iter(self.inner.as_ref())
                .expect("Expected to have an item but found none.");
            assert_eq!(
                self.parent_opid,
                input_op
                    .parent_opid
                    .expect("Expected an operation with a parent_opid."),
                "Expected parent_opid {:?} did not match operation {:#?}",
                self.parent_opid,
                input_op,
            );

            if let TraceOpContent::AdvanceInputIterator = &input_op.content {
                let input_data = self.contexts.next();

                let (_, input_op) = advance_ref_iter(self.inner.as_ref())
                    .expect("Expected to have an item but found none.");
                assert_eq!(
                    self.parent_opid,
                    input_op
                        .parent_opid
                        .expect("Expected an operation with a parent_opid."),
                    "Expected parent_opid {:?} did not match operation {:#?}",
                    self.parent_opid,
                    input_op,
                );

                if let TraceOpContent::YieldInto(context) = &input_op.content {
                    let input_context = input_data.unwrap();
                    assert_eq!(context, &input_context);

                    self.input_batch.push_back(input_context);
                } else if let TraceOpContent::InputIteratorExhausted = &input_op.content {
                    assert_eq!(None, input_data);
                } else {
                    unreachable!();
                }
            } else {
                break input_op;
            }
        };

        match &next_op.content {
            TraceOpContent::YieldFrom(YieldValue::ResolveNeighborsOuter(trace_context)) => {
                let input_context = self.input_batch.pop_front().unwrap();
                assert_eq!(trace_context, &input_context);

                let neighbors = Box::new(TraceReaderNeighborIter {
                    exhausted: false,
                    parent_iterator_opid: next_op.opid,
                    next_index: 0,
                    inner: self.inner.clone(),
                    _phantom: PhantomData,
                });
                Some((input_context, neighbors))
            }
            TraceOpContent::OutputIteratorExhausted => {
                assert_eq!(None, self.input_batch.pop_front());
                self.exhausted = true;
                None
            }
            _ => unreachable!(),
        }
    }
}

struct TraceReaderNeighborIter<'query, 'trace, Vertex>
where
    Vertex: Clone + Debug + PartialEq + Eq + Serialize + 'query,
    for<'de2> Vertex: Deserialize<'de2>,
    'trace: 'query,
{
    exhausted: bool,
    parent_iterator_opid: Opid,
    next_index: usize,
    inner: Rc<RefCell<btree_map::Iter<'trace, Opid, TraceOp<Vertex>>>>,
    _phantom: PhantomData<&'query ()>,
}

impl<'query, 'trace, Vertex> Iterator for TraceReaderNeighborIter<'query, 'trace, Vertex>
where
    Vertex: Clone + Debug + PartialEq + Eq + Serialize + 'query,
    for<'de2> Vertex: Deserialize<'de2>,
    'trace: 'query,
{
    type Item = Vertex;

    fn next(&mut self) -> Option<Self::Item> {
        let (_, trace_op) = advance_ref_iter(self.inner.as_ref())
            .expect("Expected to have an item but found none.");
        assert!(!self.exhausted);
        assert_eq!(
            self.parent_iterator_opid,
            trace_op
                .parent_opid
                .expect("Expected an operation with a parent_opid."),
            "Expected parent_opid {:?} did not match operation {:#?}",
            self.parent_iterator_opid,
            trace_op,
        );

        match &trace_op.content {
            TraceOpContent::OutputIteratorExhausted => {
                self.exhausted = true;
                None
            }
            TraceOpContent::YieldFrom(YieldValue::ResolveNeighborsInner(index, vertex)) => {
                assert_eq!(self.next_index, *index);
                self.next_index += 1;
                Some(vertex.clone())
            }
            _ => unreachable!(),
        }
    }
}

#[allow(unused_variables)]
impl<'trace, Vertex> Adapter<'trace> for TraceReaderAdapter<'trace, Vertex>
where
    Vertex: Clone + Debug + PartialEq + Eq + Serialize + 'trace,
    for<'de2> Vertex: Deserialize<'de2>,
{
    type Vertex = Vertex;

    fn resolve_starting_vertices(
        &mut self,
        edge_name: &Arc<str>,
        parameters: &EdgeParameters,
        query_info: &QueryInfo,
    ) -> VertexIterator<'trace, Self::Vertex> {
        let (root_opid, trace_op) = advance_ref_iter(self.next_op.as_ref())
            .expect("Expected a resolve_starting_vertices() call operation, but found none.");
        assert_eq!(None, trace_op.parent_opid);

        if let TraceOpContent::Call(FunctionCall::ResolveStartingVertices(vid)) = trace_op.content {
            assert_eq!(vid, query_info.origin_vid());
            assert!(query_info.origin_crossing_eid().is_none());

            Box::new(TraceReaderStartingVerticesIter {
                exhausted: false,
                parent_opid: *root_opid,
                inner: self.next_op.clone(),
            })
        } else {
            unreachable!()
        }
    }

    fn resolve_property(
        &mut self,
        contexts: ContextIterator<'trace, Self::Vertex>,
        type_name: &Arc<str>,
        property_name: &Arc<str>,
        query_info: &QueryInfo,
    ) -> ContextOutcomeIterator<'trace, Self::Vertex, FieldValue> {
        let (root_opid, trace_op) = advance_ref_iter(self.next_op.as_ref())
            .expect("Expected a resolve_property() call operation, but found none.");
        assert_eq!(None, trace_op.parent_opid);

        if let TraceOpContent::Call(FunctionCall::ResolveProperty(vid, op_type_name, property)) =
            &trace_op.content
        {
            assert_eq!(*vid, query_info.origin_vid());
            assert_eq!(op_type_name, type_name);
            assert_eq!(property, property_name);
            assert!(query_info.origin_crossing_eid().is_none());

            Box::new(TraceReaderResolvePropertiesIter {
                exhausted: false,
                parent_opid: *root_opid,
                contexts,
                input_batch: Default::default(),
                inner: self.next_op.clone(),
            })
        } else {
            unreachable!()
        }
    }

    fn resolve_neighbors(
        &mut self,
        contexts: ContextIterator<'trace, Self::Vertex>,
        type_name: &Arc<str>,
        edge_name: &Arc<str>,
        parameters: &EdgeParameters,
        query_info: &QueryInfo,
    ) -> ContextOutcomeIterator<'trace, Self::Vertex, VertexIterator<'trace, Self::Vertex>> {
        let (root_opid, trace_op) = advance_ref_iter(self.next_op.as_ref())
            .expect("Expected a resolve_property() call operation, but found none.");
        assert_eq!(None, trace_op.parent_opid);

        if let TraceOpContent::Call(FunctionCall::ResolveNeighbors(vid, op_type_name, eid)) =
            &trace_op.content
        {
            assert_eq!(*vid, query_info.origin_vid());
            assert_eq!(op_type_name, type_name);
            assert_eq!(Some(*eid), query_info.origin_crossing_eid());

            Box::new(TraceReaderResolveNeighborsIter {
                exhausted: false,
                parent_opid: *root_opid,
                contexts,
                input_batch: Default::default(),
                inner: self.next_op.clone(),
            })
        } else {
            unreachable!()
        }
    }

    fn resolve_coercion(
        &mut self,
        contexts: ContextIterator<'trace, Self::Vertex>,
        type_name: &Arc<str>,
        coerce_to_type: &Arc<str>,
        query_info: &QueryInfo,
    ) -> ContextOutcomeIterator<'trace, Self::Vertex, bool> {
        let (root_opid, trace_op) = advance_ref_iter(self.next_op.as_ref())
            .expect("Expected a resolve_coercion() call operation, but found none.");
        assert_eq!(None, trace_op.parent_opid);

        if let TraceOpContent::Call(FunctionCall::ResolveCoercion(vid, from_type, to_type)) =
            &trace_op.content
        {
            assert_eq!(*vid, query_info.origin_vid());
            assert_eq!(from_type, type_name);
            assert_eq!(to_type, coerce_to_type);
            assert!(query_info.origin_crossing_eid().is_none());

            Box::new(TraceReaderResolveCoercionIter {
                exhausted: false,
                parent_opid: *root_opid,
                contexts,
                input_batch: Default::default(),
                inner: self.next_op.clone(),
            })
        } else {
            unreachable!()
        }
    }
}

#[allow(dead_code)]
pub fn assert_interpreted_results<'query, 'trace, Vertex>(
    trace: &Trace<Vertex>,
    expected_results: &[BTreeMap<Arc<str>, FieldValue>],
    complete: bool,
) where
    Vertex: Clone + Debug + PartialEq + Eq + Serialize + 'query,
    for<'de2> Vertex: Deserialize<'de2>,
    'trace: 'query,
{
    let next_op = Rc::new(RefCell::new(trace.ops.iter()));
    let trace_reader_adapter = Rc::new(RefCell::new(TraceReaderAdapter {
        next_op: next_op.clone(),
    }));

    let query: Arc<IndexedQuery> = Arc::new(trace.ir_query.clone().try_into().unwrap());
    let arguments = Arc::new(
        trace
            .arguments
            .iter()
            .map(|(k, v)| (Arc::from(k.to_owned()), v.clone()))
            .collect(),
    );
    let mut trace_iter = interpret_ir(trace_reader_adapter, query, arguments).unwrap();
    let mut expected_iter = expected_results.iter();

    loop {
        let expected_row = expected_iter.next();
        let trace_row = trace_iter.next();

        if let Some(expected_row_content) = expected_row {
            let trace_expected_row = {
                let mut next_op_ref = next_op.borrow_mut();
                let Some((_, trace_op)) = next_op_ref.next() else {
                    panic!("Reached the end of the trace without producing result {trace_row:#?}");
                };
                let TraceOpContent::ProduceQueryResult(expected_result) = &trace_op.content else {
                    panic!("Expected the trace to produce a result {trace_row:#?} but got another type of operation instead: {trace_op:#?}");
                };
                drop(next_op_ref);

                expected_result
            };
            assert_eq!(
                trace_expected_row, expected_row_content,
                "This trace is self-inconsistent: trace produces row {trace_expected_row:#?} \
                but results have row {expected_row_content:#?}",
            );

            assert_eq!(expected_row, trace_row.as_ref());
        } else {
            if complete {
                assert_eq!(None, trace_row);
            }
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fmt::Debug,
        fs,
        path::{Path, PathBuf},
    };

    use serde::{Deserialize, Serialize};
    use trustfall_filetests_macros::parameterize;

    use crate::{
        filesystem_interpreter::FilesystemVertex,
        interpreter::replay::assert_interpreted_results,
        numbers_interpreter::NumbersVertex,
        util::{TestIRQuery, TestIRQueryResult, TestInterpreterOutputTrace},
    };

    fn check_trace<Vertex>(expected_ir: TestIRQuery, test_data: TestInterpreterOutputTrace<Vertex>)
    where
        Vertex: Debug + Clone + PartialEq + Eq + Serialize,
        for<'de> Vertex: Deserialize<'de>,
    {
        // Ensure that the trace file's IR hasn't drifted away from the IR file of the same name.
        assert_eq!(expected_ir.ir_query, test_data.trace.ir_query);
        assert_eq!(expected_ir.arguments, test_data.trace.arguments);

        assert_interpreted_results(&test_data.trace, &test_data.results, true);
    }

    fn check_filesystem_trace(expected_ir: TestIRQuery, input_data: &str) {
        match ron::from_str::<TestInterpreterOutputTrace<FilesystemVertex>>(input_data) {
            Ok(test_data) => {
                assert_eq!(expected_ir.schema_name, "filesystem");
                assert_eq!(test_data.schema_name, "filesystem");
                check_trace(expected_ir, test_data);
            }
            Err(e) => {
                unreachable!("failed to parse trace file: {e}");
            }
        }
    }

    fn check_numbers_trace(expected_ir: TestIRQuery, input_data: &str) {
        match ron::from_str::<TestInterpreterOutputTrace<NumbersVertex>>(input_data) {
            Ok(test_data) => {
                assert_eq!(expected_ir.schema_name, "numbers");
                assert_eq!(test_data.schema_name, "numbers");
                check_trace(expected_ir, test_data);
            }
            Err(e) => {
                unreachable!("failed to parse trace file: {e}");
            }
        }
    }

    #[parameterize("trustfall_core/test_data/tests/valid_queries")]
    fn parameterized_tester(base: &Path, stem: &str) {
        let mut input_path = PathBuf::from(base);
        input_path.push(format!("{stem}.trace.ron"));

        let input_data = fs::read_to_string(input_path).unwrap();

        let mut check_path = PathBuf::from(base);
        check_path.push(format!("{stem}.ir.ron"));
        let check_data = fs::read_to_string(check_path).unwrap();
        let expected_ir: TestIRQueryResult = ron::from_str(&check_data).unwrap();
        let expected_ir = expected_ir.unwrap();

        match expected_ir.schema_name.as_str() {
            "filesystem" => check_filesystem_trace(expected_ir, input_data.as_str()),
            "numbers" => check_numbers_trace(expected_ir, input_data.as_str()),
            _ => unreachable!("{}", expected_ir.schema_name),
        }
    }
}
