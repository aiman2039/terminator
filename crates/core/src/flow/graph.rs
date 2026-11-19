//! The step graph of a flow: the check for unique IDs, known `needs` and
//! no cycle, and the run order that the flow editor shows.
use super::{Draft, Flow, FlowError};
use petgraph::{
    algo::toposort,
    graph::{DiGraph, NodeIndex},
};
use std::collections::BTreeMap;

/// One step in run order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ordered {
    /// The step's index in file order.
    pub index: usize,
    /// The longest chain of needed steps before this step: 0 for a step
    /// that needs nothing. `None` for a step that can never start: it is
    /// in a cycle, or it needs an unknown step or such a step.
    pub depth: Option<usize>,
}

impl Flow {
    /// Unique IDs, known `needs` and no cycle.
    pub(super) fn check_graph(&self) -> Result<(), FlowError> {
        let mut graph = DiGraph::<&str, ()>::new();
        let mut nodes = BTreeMap::new();
        for step in &self.steps {
            let node = graph.add_node(step.id.as_str());
            if nodes.insert(step.id.as_str(), node).is_some() {
                return Err(FlowError::DuplicateId(step.id.clone()));
            }
        }
        for step in &self.steps {
            for needs in &step.needs {
                let (Some(from), Some(to)) =
                    (nodes.get(needs.as_str()), nodes.get(step.id.as_str()))
                else {
                    return Err(FlowError::UnknownNeed {
                        step: step.id.clone(),
                        needs: needs.clone(),
                    });
                };
                graph.add_edge(*from, *to, ());
            }
        }
        toposort(&graph, None).map(|_| ()).map_err(|cycle| {
            let id = graph.node_weight(cycle.node_id()).copied().unwrap_or("");
            FlowError::Cycle(id.to_owned())
        })
    }

    /// The steps in run order, each with its depth (see [`run_order`]).
    pub fn run_order(&self) -> Vec<Ordered> {
        let steps: Vec<_> = self
            .steps
            .iter()
            .map(|step| (step.id.as_str(), step.needs.as_slice()))
            .collect();
        run_order(&steps)
    }
}

impl Draft {
    /// The steps in run order, each with its depth (see [`run_order`]). A
    /// draft can have a cycle or an unknown `needs`: those steps come last.
    pub fn run_order(&self) -> Vec<Ordered> {
        let steps: Vec<_> = self
            .steps
            .iter()
            .map(|step| (step.id.as_str(), step.needs.as_slice()))
            .collect();
        run_order(&steps)
    }
}

/// Steps `(id, needs)` in run order: first the steps that need nothing,
/// then one depth at a time the steps whose needed steps all came before,
/// each depth in file order. The steps that can never start come last, in
/// file order. When two steps have one ID, `needs` names the first.
fn run_order(steps: &[(&str, &[String])]) -> Vec<Ordered> {
    let mut graph = DiGraph::<(), ()>::new();
    let nodes: Vec<NodeIndex> = steps.iter().map(|_| graph.add_node(())).collect();
    let mut first = BTreeMap::new();
    for (&(id, _), &node) in steps.iter().zip(&nodes) {
        first.entry(id).or_insert(node);
    }
    // The needs of each step that are not placed yet; `None` when one of
    // them is unknown.
    let mut open: Vec<Option<usize>> = Vec::with_capacity(steps.len());
    for (&(_, needs), &node) in steps.iter().zip(&nodes) {
        let mut count = Some(0_usize);
        for needs in needs {
            match first.get(needs.as_str()) {
                Some(&from) => {
                    graph.add_edge(from, node, ());
                    count = count.map(|count| count.saturating_add(1));
                }
                None => count = None,
            }
        }
        open.push(count);
    }
    let mut order = Vec::with_capacity(steps.len());
    let mut placed = vec![false; steps.len()];
    let mut layer: Vec<usize> = (0..steps.len())
        .filter(|index| open.get(*index) == Some(&Some(0)))
        .collect();
    let mut depth = 0_usize;
    while !layer.is_empty() {
        let mut next = Vec::new();
        for &index in &layer {
            order.push(Ordered {
                index,
                depth: Some(depth),
            });
            if let Some(slot) = placed.get_mut(index) {
                *slot = true;
            }
            let freed = graph
                .neighbors(NodeIndex::new(index))
                .map(|after| after.index())
                .filter(|&after| placed_need(&mut open, after));
            next.extend(freed);
        }
        next.sort_unstable();
        next.dedup();
        layer = next;
        depth = depth.saturating_add(1);
    }
    let never = placed.iter().enumerate().filter(|(_, placed)| !**placed);
    order.extend(never.map(|(index, _)| Ordered { index, depth: None }));
    order
}

/// One need of step `index` is placed: true when it was the last one.
fn placed_need(open: &mut [Option<usize>], index: usize) -> bool {
    let Some(Some(count)) = open.get_mut(index) else {
        return false;
    };
    *count = count.saturating_sub(1);
    *count == 0
}
