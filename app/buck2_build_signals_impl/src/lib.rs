/*
 * Copyright (c) Meta Platforms, Inc. and affiliates.
 *
 * This source code is licensed under both the MIT license found in the
 * LICENSE-MIT file in the root directory of this source tree and the Apache
 * License, Version 2.0 found in the LICENSE-APACHE file in the root directory
 * of this source tree.
 */

use std::any::Any;
use std::collections::HashMap;
use std::collections::HashSet;
use std::fmt;
use std::fmt::Display;
use std::future::Future;
use std::hash::Hash;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use allocative::Allocative;
use anyhow::Context as _;
use buck2_artifact::artifact::build_artifact::BuildArtifact;
use buck2_build_api::actions::calculation::BuildKey;
use buck2_build_api::actions::calculation::BuildKeyActivationData;
use buck2_build_api::actions::RegisteredAction;
use buck2_build_api::analysis::calculation::AnalysisKey;
use buck2_build_api::analysis::calculation::AnalysisKeyActivationData;
use buck2_build_api::artifact_groups::calculation::EnsureProjectedArtifactKey;
use buck2_build_api::artifact_groups::calculation::EnsureTransitiveSetProjectionKey;
use buck2_build_api::artifact_groups::ArtifactGroup;
use buck2_build_api::artifact_groups::ResolvedArtifactGroup;
use buck2_build_api::build_listener::BuildSignals;
use buck2_build_api::build_listener::BuildSignalsInstaller;
use buck2_build_api::build_listener::NodeDuration;
use buck2_build_api::deferred::calculation::DeferredCompute;
use buck2_build_api::deferred::calculation::DeferredResolve;
use buck2_build_api::nodes::calculation::ConfiguredTargetNodeKey;
use buck2_core::package::PackageLabel;
use buck2_core::soft_error;
use buck2_core::target::label::ConfiguredTargetLabel;
use buck2_critical_path::compute_critical_path_potentials;
use buck2_critical_path::GraphBuilder;
use buck2_critical_path::OptionalVertexId;
use buck2_critical_path::PushError;
use buck2_data::ToProtoMessage;
use buck2_events::dispatch::instant_event;
use buck2_events::dispatch::with_dispatcher_async;
use buck2_events::dispatch::EventDispatcher;
use buck2_events::metadata;
use buck2_events::span::SpanId;
use buck2_interpreter_for_build::interpreter::calculation::IntepreterResultsKeyActivationData;
use buck2_interpreter_for_build::interpreter::calculation::InterpreterResultsKey;
use buck2_node::nodes::eval_result::EvaluationResult;
use derive_more::From;
use dice::ActivationData;
use dice::ActivationTracker;
use dupe::Dupe;
use dupe::OptionDupedExt;
use gazebo::prelude::VecExt;
use itertools::Itertools;
use smallvec::SmallVec;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::JoinHandle;
use tokio_stream::wrappers::UnboundedReceiverStream;
use tokio_stream::StreamExt;

/// A node in our critical path graph.
#[derive(Hash, Eq, PartialEq, Clone, Dupe, Debug, From)]
enum NodeKey {
    // Those are DICE keys.
    BuildKey(BuildKey),
    AnalysisKey(AnalysisKey),
    EnsureProjectedArtifactKey(EnsureProjectedArtifactKey),
    EnsureTransitiveSetProjectionKey(EnsureTransitiveSetProjectionKey),
    DeferredCompute(DeferredCompute),
    DeferredResolve(DeferredResolve),
    ConfiguredTargetNodeKey(ConfiguredTargetNodeKey),
    InterpreterResultsKey(InterpreterResultsKey),

    // This one is not a DICE key.
    Materialization(BuildArtifact),
}

impl NodeKey {
    fn from_any(key: &dyn Any) -> Option<Self> {
        let key = if let Some(key) = key.downcast_ref::<BuildKey>() {
            Self::BuildKey(key.dupe())
        } else if let Some(key) = key.downcast_ref::<AnalysisKey>() {
            Self::AnalysisKey(key.dupe())
        } else if let Some(key) = key.downcast_ref::<EnsureProjectedArtifactKey>() {
            Self::EnsureProjectedArtifactKey(key.dupe())
        } else if let Some(key) = key.downcast_ref::<EnsureTransitiveSetProjectionKey>() {
            Self::EnsureTransitiveSetProjectionKey(key.dupe())
        } else if let Some(key) = key.downcast_ref::<DeferredCompute>() {
            Self::DeferredCompute(key.dupe())
        } else if let Some(key) = key.downcast_ref::<DeferredResolve>() {
            Self::DeferredResolve(key.dupe())
        } else if let Some(key) = key.downcast_ref::<ConfiguredTargetNodeKey>() {
            Self::ConfiguredTargetNodeKey(key.dupe())
        } else if let Some(key) = key.downcast_ref::<InterpreterResultsKey>() {
            Self::InterpreterResultsKey(key.dupe())
        } else {
            return None;
        };

        Some(key)
    }
}

impl fmt::Display for NodeKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BuildKey(k) => write!(f, "BuildKey({})", k),
            Self::AnalysisKey(k) => write!(f, "AnalysisKey({})", k),
            Self::EnsureProjectedArtifactKey(k) => write!(f, "EnsureProjectedArtifactKey({})", k),
            Self::EnsureTransitiveSetProjectionKey(k) => {
                write!(f, "EnsureTransitiveSetProjectionKey({})", k)
            }
            Self::DeferredCompute(k) => write!(f, "DeferredCompute({})", k),
            Self::DeferredResolve(k) => write!(f, "DeferredResolve({})", k),
            Self::ConfiguredTargetNodeKey(k) => write!(f, "ConfiguredTargetNodeKey({})", k),
            Self::InterpreterResultsKey(k) => write!(f, "InterpreterResultsKey({})", k),
            Self::Materialization(k) => write!(f, "Materialization({})", k),
        }
    }
}

struct TopLevelTargetSignal {
    pub label: ConfiguredTargetLabel,
    pub artifacts: Vec<ArtifactGroup>,
}

struct FinalMaterializationSignal {
    pub artifact: BuildArtifact,
    pub duration: NodeDuration,
    pub span_id: Option<SpanId>,
}

/* These signals are distinct from the main Buck event bus because some
 * analysis needs access to the entire build graph, and serializing the
 * entire build graph isn't feasible - therefore, we have these signals
 * with an unserializable but lightweight handle on a RegisteredAction.
 */
#[derive(From)]
enum BuildSignal {
    Evaluation(Evaluation),
    TopLevelTarget(TopLevelTargetSignal),
    FinalMaterialization(FinalMaterializationSignal),
    BuildFinished,
}

/// Data for a BuildSignal that is the result of a DICE key evaluation.
pub struct Evaluation {
    /// The key we evaluated.
    key: NodeKey,
    /// The duration. By default this'll be zero, unless activation data says otherwise.
    duration: NodeDuration,
    /// The dependencies.
    dep_keys: Vec<NodeKey>,
    /// Spans that correspond to this key. We use this when producing a chrome trace.
    spans: SmallVec<[SpanId; 1]>,

    // NOTE: The fields below aren't usually going to be both set, but it doesn't really hurt (for
    // now) to have them not tied to the right variant.
    /// The RegisteredAction that corresponds to this Evaluation (this will only be present for
    /// NodeKey::BuildKey).
    action: Option<Arc<RegisteredAction>>,

    /// The Load result that corresponds to this Evaluation (this will only be pesent for
    /// InterpreterResultsKey).
    load_result: Option<Arc<EvaluationResult>>,
}

pub struct BuildSignalSender {
    sender: UnboundedSender<BuildSignal>,
}

impl BuildSignals for BuildSignalSender {
    fn top_level_target(&self, label: ConfiguredTargetLabel, artifacts: Vec<ArtifactGroup>) {
        let _ignored = self
            .sender
            .send(TopLevelTargetSignal { label, artifacts }.into());
    }

    fn final_materialization(
        &self,
        artifact: BuildArtifact,
        duration: NodeDuration,
        span_id: Option<SpanId>,
    ) {
        let _ignored = self.sender.send(
            FinalMaterializationSignal {
                artifact,
                duration,
                span_id,
            }
            .into(),
        );
    }

    fn build_finished(&self) {
        let _ignored = self.sender.send(BuildSignal::BuildFinished);
    }
}

impl ActivationTracker for BuildSignalSender {
    /// We received a DICE key. Check if it's one of the keys we care about (i.e. can we downcast
    /// it to NodeKey?), and then if that's the case, extract its dependencies and activation data
    /// (if any).
    fn key_activated(
        &self,
        key: &dyn Any,
        deps: &mut dyn Iterator<Item = &dyn Any>,
        activation_data: ActivationData,
    ) {
        let key = match NodeKey::from_any(key) {
            Some(key) => key,
            None => return,
        };

        let mut signal = Evaluation {
            key,
            action: None,
            duration: NodeDuration::zero(),
            dep_keys: deps.into_iter().filter_map(NodeKey::from_any).collect(),
            spans: Default::default(),
            load_result: None,
        };

        /// Given an Option containing an Any, take it if and only if it contains a T.
        fn downcast_and_take<T: 'static>(
            data: &mut Option<Box<dyn Any + Send + Sync + 'static>>,
        ) -> Option<T> {
            if data.as_ref().map(|d| d.is::<T>()) != Some(true) {
                return None;
            }

            // Unwrap safety: we just checked that the option is occupied and the type matches
            Some(*data.take().unwrap().downcast().ok().unwrap())
        }

        if let ActivationData::Evaluated(mut activation_data) = activation_data {
            if let Some(BuildKeyActivationData {
                action,
                duration,
                spans,
            }) = downcast_and_take(&mut activation_data)
            {
                signal.action = Some(action);
                signal.duration = duration;
                signal.spans = spans;
            } else if let Some(AnalysisKeyActivationData { duration, spans }) =
                downcast_and_take(&mut activation_data)
            {
                signal.duration = NodeDuration {
                    user: duration,
                    total: duration,
                };
                signal.spans = spans;
            } else if let Some(IntepreterResultsKeyActivationData {
                duration,
                result,
                spans,
            }) = downcast_and_take(&mut activation_data)
            {
                signal.duration = NodeDuration {
                    user: duration,
                    total: duration,
                };

                signal.load_result = result.ok();
                signal.spans = spans;
            }
        }

        let _ignored = self.sender.send(signal.into());
    }
}

#[derive(Clone, Dupe)]
struct CriticalPathNode<TKey: Eq, TValue> {
    /// The aggregated duration of this critical path.
    pub duration: Duration,
    /// The value of this node. If None, this node just won't be included when displaying.
    pub value: TValue,
    pub prev: Option<TKey>,
}

struct BuildSignalReceiver<T> {
    receiver: UnboundedReceiverStream<BuildSignal>,
    // Maps a PackageLabel to the first PackageLabel that had an edge to it. When that PackageLabel
    // shows up, we'll give it a dependency on said first PackageLabel that had an edge to it, which
    // is how we discovered its existence.
    first_edge_to_load: HashMap<PackageLabel, PackageLabel>,
    backend: T,
}

fn extract_critical_path<TKey: Hash + Eq, TValue>(
    predecessors: &HashMap<TKey, CriticalPathNode<TKey, TValue>>,
) -> anyhow::Result<Vec<(&TKey, &TValue, Duration)>>
where
    TKey: Display,
{
    let mut tail = predecessors
        .iter()
        .max_by_key(|(_key, data)| data.duration)
        .map(|q| q.0);

    let mut path = vec![];
    let mut visited = HashSet::new();

    while let Some(v) = tail.take() {
        if !visited.insert(v) {
            return Err(anyhow::anyhow!(
                "Cycle in critical path: visited {} twice",
                v
            ));
        }

        tail = predecessors.get(v).and_then(|node| {
            path.push((v, &node.value, node.duration));
            node.prev.as_ref()
        });
    }

    // Take differences of adjacent elements to recover action time from cumulative sum.
    path.reverse();
    for i in (1..path.len()).rev() {
        path[i].2 = path[i].2.saturating_sub(path[i - 1].2);
    }

    Ok(path)
}

impl<T> BuildSignalReceiver<T>
where
    T: BuildListenerBackend,
{
    fn new(receiver: UnboundedReceiver<BuildSignal>, backend: T) -> Self {
        Self {
            receiver: UnboundedReceiverStream::new(receiver),
            backend,
            first_edge_to_load: HashMap::new(),
        }
    }

    pub async fn run_and_log(mut self) -> anyhow::Result<()> {
        while let Some(event) = self.receiver.next().await {
            match event {
                BuildSignal::Evaluation(eval) => self.process_evaluation(eval),
                BuildSignal::TopLevelTarget(top_level) => {
                    self.process_top_level_target(top_level)?
                }
                BuildSignal::FinalMaterialization(final_materialization) => {
                    self.process_final_materialization(final_materialization)?
                }
                BuildSignal::BuildFinished => break,
            }
        }

        let now = Instant::now();

        let BuildInfo {
            critical_path,
            num_nodes,
            num_edges,
        } = self.backend.finish()?;

        let compute_elapsed = now.elapsed();

        let meta_entry_data = NodeData {
            action: None,
            duration: NodeDuration {
                user: Duration::ZERO,
                total: compute_elapsed,
            },
            span_ids: Default::default(),
        };

        let meta_entry = (
            buck2_data::critical_path_entry2::ComputeCriticalPath {}.into(),
            &meta_entry_data,
            &Some(compute_elapsed),
        );

        let critical_path2 = critical_path
            .iter()
            .filter_map(|(key, data, potential_improvement)| {
                let entry: buck2_data::critical_path_entry2::Entry = match key {
                    NodeKey::BuildKey(key) => {
                        let owner = key.0.owner().to_proto().into();

                        // If we have a NodeKey that's an ActionKey we'd expect to have an `action`
                        // in our data (unless we didn't actually run it because of e.g. early
                        // cutoff, in which case omitting it is what we want).
                        let action = data.action.as_ref()?;

                        buck2_data::critical_path_entry2::ActionExecution {
                            owner: Some(owner),
                            name: Some(buck2_data::ActionName {
                                category: action.category().as_str().to_owned(),
                                identifier: action.identifier().unwrap_or("").to_owned(),
                            }),
                        }
                        .into()
                    }
                    NodeKey::AnalysisKey(key) => buck2_data::critical_path_entry2::Analysis {
                        target: Some(key.0.as_proto().into()),
                    }
                    .into(),
                    NodeKey::Materialization(key) => {
                        let owner = key.key().owner().to_proto().into();

                        buck2_data::critical_path_entry2::Materialization {
                            owner: Some(owner),
                            path: key.get_path().path().to_string(),
                        }
                        .into()
                    }
                    NodeKey::InterpreterResultsKey(key) => buck2_data::critical_path_entry2::Load {
                        package: key.0.to_string(),
                    }
                    .into(),
                    NodeKey::EnsureProjectedArtifactKey(..) => return None,
                    NodeKey::EnsureTransitiveSetProjectionKey(..) => return None,
                    NodeKey::DeferredCompute(..) => return None,
                    NodeKey::DeferredResolve(..) => return None,
                    NodeKey::ConfiguredTargetNodeKey(..) => return None,
                };

                Some((entry, data, potential_improvement))
            })
            .chain(std::iter::once(meta_entry))
            .map(|(entry, data, potential_improvement)| {
                anyhow::Ok(buck2_data::CriticalPathEntry2 {
                    span_ids: data
                        .span_ids
                        .iter()
                        .map(|span_id| (*span_id).into())
                        .collect(),
                    duration: Some(data.duration.critical_path_duration().try_into()?),
                    user_duration: Some(data.duration.user.try_into()?),
                    total_duration: Some(data.duration.total.try_into()?),
                    potential_improvement_duration: potential_improvement
                        .map(|p| p.try_into())
                        .transpose()?,
                    entry: Some(entry),
                })
            })
            .collect::<Result<Vec<_>, _>>()?;

        instant_event(buck2_data::BuildGraphExecutionInfo {
            critical_path: Vec::new(),
            critical_path2,
            metadata: metadata::collect(),
            num_nodes,
            num_edges,
            uses_total_duration: true,
            backend_name: Some(T::name().to_string()),
        });
        Ok(())
    }

    /// Receive an Evaluation. Do a little enrichment if it's a load, then pass through to the
    /// underying backend.
    fn process_evaluation(&mut self, mut evaluation: Evaluation) {
        self.enrich_load(&mut evaluation);

        self.backend.process_node(
            evaluation.key,
            evaluation.action,
            evaluation.duration,
            evaluation.dep_keys.into_iter(),
            evaluation.spans,
        );
    }

    /// If the evaluation is a load (InterpreterResultsKey) and carries a load_result, then inject
    /// some extra edges that indicate which packages have now become visibile as a result of this
    /// load.
    fn enrich_load(&mut self, evaluation: &mut Evaluation) {
        let pkg = match &evaluation.key {
            NodeKey::InterpreterResultsKey(InterpreterResultsKey(pkg)) => pkg,
            _ => return,
        };

        if let Some(load_result) = &evaluation.load_result {
            let deps_pkg = load_result
                .targets()
                .values()
                .flat_map(|target| target.deps().map(|t| t.pkg()))
                .unique()
                .map(|pkg| pkg.dupe());

            for dep_pkg in deps_pkg {
                if dep_pkg == *pkg {
                    continue;
                }

                self.first_edge_to_load
                    .entry(dep_pkg)
                    .or_insert_with(|| pkg.dupe());
            }
        }

        let first_edge = self.first_edge_to_load.get(pkg);

        if let Some(first_edge) = first_edge {
            evaluation
                .dep_keys
                .push(NodeKey::InterpreterResultsKey(InterpreterResultsKey(
                    first_edge.dupe(),
                )));
        }
    }

    // TODO: We would need something similar with anon targets.
    fn process_top_level_target(
        &mut self,
        top_level: TopLevelTargetSignal,
    ) -> Result<(), anyhow::Error> {
        let artifact_keys =
            top_level
                .artifacts
                .into_iter()
                .filter_map(|dep| match dep.assert_resolved() {
                    ResolvedArtifactGroup::Artifact(artifact) => artifact
                        .action_key()
                        .duped()
                        .map(BuildKey)
                        .map(NodeKey::BuildKey),
                    ResolvedArtifactGroup::TransitiveSetProjection(key) => {
                        Some(NodeKey::EnsureTransitiveSetProjectionKey(
                            EnsureTransitiveSetProjectionKey(key.dupe()),
                        ))
                    }
                });

        self.backend.process_top_level_target(
            NodeKey::AnalysisKey(AnalysisKey(top_level.label)),
            artifact_keys,
        );

        Ok(())
    }

    fn process_final_materialization(
        &mut self,
        materialization: FinalMaterializationSignal,
    ) -> Result<(), anyhow::Error> {
        let dep = NodeKey::BuildKey(BuildKey(materialization.artifact.key().dupe()));

        self.backend.process_node(
            NodeKey::Materialization(materialization.artifact),
            None,
            materialization.duration,
            std::iter::once(dep),
            materialization.span_id.into_iter().collect(),
        );

        Ok(())
    }
}

trait BuildListenerBackend {
    fn process_node(
        &mut self,
        key: NodeKey,
        value: Option<Arc<RegisteredAction>>,
        duration: NodeDuration,
        dep_keys: impl Iterator<Item = NodeKey>,
        span_ids: SmallVec<[SpanId; 1]>,
    );

    fn process_top_level_target(
        &mut self,
        analysis: NodeKey,
        artifacts: impl Iterator<Item = NodeKey>,
    );

    fn finish(self) -> anyhow::Result<BuildInfo>;

    fn name() -> CriticalPathBackendName;
}

pub struct BuildInfo {
    // Node, its data, and its potential for improvement
    critical_path: Vec<(NodeKey, NodeData, Option<Duration>)>,
    num_nodes: u64,
    num_edges: u64,
}

struct DefaultBackend {
    predecessors: HashMap<NodeKey, CriticalPathNode<NodeKey, NodeData>>,
    num_nodes: u64,
    num_edges: u64,
}

impl DefaultBackend {
    fn new() -> Self {
        Self {
            predecessors: HashMap::new(),
            num_nodes: 0,
            num_edges: 0,
        }
    }
}

impl BuildListenerBackend for DefaultBackend {
    fn process_node(
        &mut self,
        key: NodeKey,
        value: Option<Arc<RegisteredAction>>,
        duration: NodeDuration,
        dep_keys: impl Iterator<Item = NodeKey>,
        span_ids: SmallVec<[SpanId; 1]>,
    ) {
        let longest_ancestor = dep_keys
            .unique()
            .filter_map(|node_key| {
                self.num_edges += 1;
                let node_data = self.predecessors.get(&node_key)?;
                Some((node_key, node_data.duration))
            })
            .max_by_key(|d| d.1);

        let value = NodeData {
            action: value,
            duration,
            span_ids,
        };

        let node = match longest_ancestor {
            Some((key, ancestor_duration)) => CriticalPathNode {
                prev: Some(key.dupe()),
                value,
                duration: ancestor_duration + duration.critical_path_duration(),
            },
            None => CriticalPathNode {
                prev: None,
                value,
                duration: duration.critical_path_duration(),
            },
        };

        self.num_nodes += 1;
        self.predecessors.insert(key, node);
    }

    fn process_top_level_target(
        &mut self,
        _analysis: NodeKey,
        _artifacts: impl Iterator<Item = NodeKey>,
    ) {
    }

    fn finish(self) -> anyhow::Result<BuildInfo> {
        let critical_path = extract_critical_path(&self.predecessors)
            .context("Error extracting critical path")?
            .into_map(|(key, data, _duration)| (key.dupe(), data.clone(), None));

        Ok(BuildInfo {
            critical_path,
            num_nodes: self.num_nodes,
            num_edges: self.num_edges,
        })
    }

    fn name() -> CriticalPathBackendName {
        CriticalPathBackendName::Default
    }
}

/// An implementation of critical path that uses a longest-paths graph in order to produce
/// potential savings in addition to the critical path.
struct LongestPathGraphBackend {
    builder: anyhow::Result<GraphBuilder<NodeKey, NodeData>>,
    top_level_analysis: Vec<VisibilityEdge>,
}

#[derive(Clone)]
struct NodeData {
    action: Option<Arc<RegisteredAction>>,
    duration: NodeDuration,
    span_ids: SmallVec<[SpanId; 1]>,
}

/// Represents nodes that block us "seeing" other parts of the graph until they finish evaluating.
struct VisibilityEdge {
    node: NodeKey,
    makes_visible: Vec<NodeKey>,
}

impl LongestPathGraphBackend {
    fn new() -> Self {
        Self {
            builder: Ok(GraphBuilder::new()),
            top_level_analysis: Vec::new(),
        }
    }
}

impl BuildListenerBackend for LongestPathGraphBackend {
    fn process_node(
        &mut self,
        key: NodeKey,
        action: Option<Arc<RegisteredAction>>,
        duration: NodeDuration,
        dep_keys: impl Iterator<Item = NodeKey>,
        span_ids: SmallVec<[SpanId; 1]>,
    ) {
        let builder = match self.builder.as_mut() {
            Ok(b) => b,
            Err(..) => return,
        };

        let res = builder.push(
            key,
            dep_keys,
            NodeData {
                action,
                duration,
                span_ids,
            },
        );

        let res = res.or_else(|err| match err {
            e @ PushError::Overflow => Err(e.into()),
            e @ PushError::DuplicateKey { .. } => {
                soft_error!("critical_path_duplicate_key", e.into(), quiet: true)?;
                anyhow::Ok(())
            }
        });

        match res {
            Ok(()) => {}
            Err(e) => self.builder = Err(e),
        }
    }

    fn process_top_level_target(
        &mut self,
        analysis: NodeKey,
        artifacts: impl Iterator<Item = NodeKey>,
    ) {
        self.top_level_analysis.push(VisibilityEdge {
            node: analysis,
            makes_visible: artifacts.collect(),
        })
    }

    fn finish(self) -> anyhow::Result<BuildInfo> {
        let (graph, keys, mut data) = {
            let (graph, keys, data) = self.builder?.finish();

            let mut first_analysis = graph.allocate_vertex_data(OptionalVertexId::none());
            let mut n = 0;

            for visibility in &self.top_level_analysis {
                let analysis = &visibility.node;
                let artifacts = &visibility.makes_visible;

                let analysis = match keys.get(analysis) {
                    Some(k) => k,
                    None => continue, // Nothing depends on this,
                };

                let mut queue = Vec::new();

                // We have an analysis and a set of artifacts that we decided to build after having
                // evaluated this analysis. So, traverse all of those artifacts' dependencies, and
                // label them as depending on this top level analysis (but we only do that once).
                // Concretely, this expresses the idea that we only started knowing we cared about
                // those artifacts once we finished that analysis.

                for artifact in artifacts {
                    let artifact = match keys.get(artifact) {
                        Some(a) => a,
                        None => {
                            // Not built. Unexpected, but we don't report signals in all failure cases so that can happen.
                            continue;
                        }
                    };

                    queue.push(artifact);

                    while let Some(i) = queue.pop() {
                        if first_analysis[i].is_some() {
                            continue;
                        }

                        // We only traverse edges to things that produce artifacts here.
                        match keys[i] {
                            NodeKey::BuildKey(..)
                            | NodeKey::EnsureTransitiveSetProjectionKey(..)
                            | NodeKey::EnsureProjectedArtifactKey(..) => {}
                            _ => {
                                continue;
                            }
                        };

                        first_analysis[i] = analysis.into();
                        queue.extend(graph.iter_edges(i));
                        n += 1;
                    }
                }
            }

            let graph = graph
                .add_edges(&first_analysis, n)
                .context("Error adding first_analysis edges to graph")?;

            (graph, keys, data)
        };

        let durations = data.try_map_ref(|d| {
            d.duration
                .critical_path_duration()
                .as_micros()
                .try_into()
                .context("Duration `as_micros()` exceeds u64")
        })?;

        let (critical_path, critical_path_cost, replacement_durations) =
            compute_critical_path_potentials(&graph, &durations)
                .context("Error computing critical path potentials")?;

        drop(durations);

        let critical_path = critical_path
            .iter()
            .map(|(cp_idx, vertex_idx)| {
                let vertex_idx = *vertex_idx;
                let key = keys[vertex_idx].dupe();

                // OK to replace `data` with empty things here because we know that we will not access
                // the same index twice.
                let data = std::mem::replace(
                    &mut data[vertex_idx],
                    NodeData {
                        action: None,
                        duration: NodeDuration::zero(),
                        span_ids: Default::default(),
                    },
                );

                let potential = critical_path_cost.runtime - replacement_durations[cp_idx].runtime;

                (key, data, Some(Duration::from_micros(potential)))
            })
            .collect();

        Ok(BuildInfo {
            critical_path,
            num_nodes: graph.vertices_count() as _,
            num_edges: graph.edges_count() as _,
        })
    }

    fn name() -> CriticalPathBackendName {
        CriticalPathBackendName::LongestPathGraph
    }
}

fn start_listener(
    events: EventDispatcher,
    backend: impl BuildListenerBackend + Send + 'static,
) -> (BuildSignalsInstaller, JoinHandle<anyhow::Result<()>>) {
    let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
    let sender = BuildSignalSender { sender };

    let listener = BuildSignalReceiver::new(receiver, backend);
    let receiver_task_handle = tokio::spawn(with_dispatcher_async(events.dupe(), async move {
        listener.run_and_log().await
    }));

    let sender = Arc::new(sender);

    let installer = BuildSignalsInstaller {
        build_signals: sender.dupe() as _,
        activation_tracker: sender as _,
    };

    (installer, receiver_task_handle)
}

#[derive(Copy, Clone, Dupe, derive_more::Display, Allocative)]
pub enum CriticalPathBackendName {
    #[display(fmt = "longest-path-graph")]
    LongestPathGraph,
    #[display(fmt = "default")]
    Default,
}

impl FromStr for CriticalPathBackendName {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s == "longest-path-graph" {
            return Ok(Self::LongestPathGraph);
        }

        if s == "default" {
            return Ok(Self::Default);
        }

        Err(anyhow::anyhow!("Invalid backend name: `{}`", s))
    }
}

/// Creates a Build Listener signal pair and invokes the given asynchronous function with the send-end of the signal
/// sender.
///
/// Build listeners in this module operate by creating a matched pair of signal senders and signal receivers. Senders
/// are Dupe and allow for arbitrarily many writeres. Receivers are not Dupe and are expected to be driven by a single
/// thread. This implies that, in order for the receiver to function correctly and dispatch to build listeners, it must
/// be run in a background task that is periodically polled.
///
/// This function arranges for a background task to be spawned that drives the receiver, while invoking the called
/// function with a live BuildSignalSender that can be used to send events to the listening receiver. Upon return of
/// `scope`, the sender terminates the receiver by sending a `BuildFinished` signal and joins the receiver task.
pub async fn scope<F, R, Fut>(
    events: EventDispatcher,
    backend: CriticalPathBackendName,
    func: F,
) -> anyhow::Result<R>
where
    F: FnOnce(BuildSignalsInstaller) -> Fut,
    Fut: Future<Output = anyhow::Result<R>>,
{
    let (installer, handle) = match backend {
        CriticalPathBackendName::LongestPathGraph => {
            start_listener(events, LongestPathGraphBackend::new())
        }
        CriticalPathBackendName::Default => start_listener(events, DefaultBackend::new()),
    };
    let result = func(installer.dupe()).await;
    installer.build_signals.build_finished();
    let res = handle
        .await
        .context("Error joining critical path task")?
        .context("Error computing critical path");
    if let Err(e) = res {
        soft_error!("critical_path_computation_failed", e)?;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    type CriticalPathMap = HashMap<i32, CriticalPathNode<i32, Option<i32>>>;

    fn cp_insert(
        predecessors: &mut CriticalPathMap,
        key: i32,
        prev: Option<i32>,
        duration: Duration,
    ) {
        predecessors.insert(
            key,
            CriticalPathNode {
                duration,
                value: Some(key),
                prev,
            },
        );
    }
    #[test]
    fn empty_path() {
        let predecessors = CriticalPathMap::new();
        assert_eq!(extract_critical_path(&predecessors).unwrap(), vec![]);
    }

    #[test]
    fn unit_path() {
        let mut predecessors = CriticalPathMap::new();
        cp_insert(&mut predecessors, 1, None, Duration::from_secs(3));
        assert_eq!(
            extract_critical_path(&predecessors).unwrap(),
            vec![(&1, &Some(1), Duration::from_secs(3))],
        );
    }

    #[test]
    fn long_path() {
        let mut predecessors = HashMap::new();
        /*   -> 1 -> 2 -> 3
         *   5s   6s   7s
         *
         *      1 -> 4
         *        9s
         */
        cp_insert(&mut predecessors, 1, None, Duration::from_secs(5));
        cp_insert(&mut predecessors, 2, Some(1), Duration::from_secs(11));
        cp_insert(&mut predecessors, 3, Some(2), Duration::from_secs(18));
        cp_insert(&mut predecessors, 4, Some(1), Duration::from_secs(14));
        assert_eq!(
            extract_critical_path(&predecessors).unwrap(),
            vec![
                (&1, &Some(1), Duration::from_secs(5)),
                (&2, &Some(2), Duration::from_secs(6)),
                (&3, &Some(3), Duration::from_secs(7)),
            ],
        );
    }

    #[test]
    fn cycle_path() {
        let mut predecessors = HashMap::new();
        cp_insert(&mut predecessors, 1, Some(2), Duration::from_secs(5));
        cp_insert(&mut predecessors, 2, Some(1), Duration::from_secs(11));
        assert!(extract_critical_path(&predecessors).is_err());
    }
}
