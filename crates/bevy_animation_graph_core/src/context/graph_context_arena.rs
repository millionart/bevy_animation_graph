use bevy::{asset::AssetId, platform::collections::HashMap, reflect::Reflect};

use crate::{
    animation_graph::{AnimationGraph, NodeId},
    context::graph_context::GraphState,
    state_machine::low_level::LowLevelStateId,
};

#[derive(Reflect, Clone, Copy, Debug, Eq, PartialEq, Hash, Default)]
pub struct GraphContextId(usize);

#[derive(Reflect, Clone, Debug, Eq, PartialEq, Hash)]
pub struct SubContextId {
    pub ctx_id: GraphContextId,
    pub node_id: NodeId,
    pub state_id: Option<LowLevelStateId>,
}

#[derive(Reflect, Clone, Debug)]
#[reflect(Clone)]
pub struct GraphContextArena {
    contexts: Vec<GraphState>,
    hierarchy: HashMap<SubContextId, GraphContextId>,
    top_level_context: GraphContextId,
    frame_index: u64,
}

impl GraphContextArena {
    pub fn new(graph_id: AssetId<AnimationGraph>) -> Self {
        Self {
            contexts: vec![GraphState::new(graph_id)],
            hierarchy: HashMap::default(),
            top_level_context: GraphContextId(0),
            frame_index: 0,
        }
    }

    pub fn iter_context_ids(&self) -> impl Iterator<Item = GraphContextId> {
        (0..self.contexts.len()).map(GraphContextId)
    }

    fn new_context(&mut self, graph_id: AssetId<AnimationGraph>) -> GraphContextId {
        self.contexts.push(GraphState::new(graph_id));

        GraphContextId(self.contexts.len() - 1)
    }

    pub fn get_context(&self, id: GraphContextId) -> Option<&GraphState> {
        self.contexts.get(id.0)
    }

    pub fn next_frame(&mut self) {
        for context in self.contexts.iter_mut() {
            context.next_frame();
        }
        self.frame_index = self.frame_index.wrapping_add(1);
    }

    pub fn frame_index(&self) -> u64 {
        self.frame_index
    }

    pub fn get_context_mut(&mut self, id: GraphContextId) -> Option<&mut GraphState> {
        self.contexts.get_mut(id.0)
    }

    pub fn get_toplevel(&self) -> &GraphState {
        self.get_context(self.get_toplevel_id()).unwrap()
    }

    pub fn get_toplevel_mut(&mut self) -> &mut GraphState {
        self.get_context_mut(self.get_toplevel_id()).unwrap()
    }

    pub fn get_toplevel_id(&self) -> GraphContextId {
        self.top_level_context
    }

    pub fn context_exists(&self, id: GraphContextId) -> bool {
        id.0 < self.contexts.len()
    }

    pub(super) fn get_sub_context_or_insert_default(
        &mut self,
        subctx_id: SubContextId,
        subgraph_id: AssetId<AnimationGraph>,
    ) -> GraphContextId {
        if !self.context_exists(subctx_id.ctx_id) {
            panic!("Context does not exist");
        }

        if !self.hierarchy.contains_key(&subctx_id) {
            let child_node_id = self.new_context(subgraph_id);
            self.hierarchy.insert(subctx_id.clone(), child_node_id);
        }

        let context_id = *self.hierarchy.get(&subctx_id).unwrap();
        if self.contexts[context_id.0].get_graph_id() != subgraph_id {
            let mut pending = vec![context_id];
            while let Some(current) = pending.pop() {
                let graph_id = if current == context_id { subgraph_id } else { self.contexts[current.0].get_graph_id() };
                self.contexts[current.0] = GraphState::new(graph_id);
                pending.extend(self.hierarchy.iter().filter_map(|(parent, child)| (parent.ctx_id == current).then_some(*child)));
            }
        }
        context_id
    }
}

#[derive(Clone)]
pub struct GraphContextArenaRef {
    context: *mut GraphContextArena,
}

impl From<&mut GraphContextArena> for GraphContextArenaRef {
    fn from(value: &mut GraphContextArena) -> Self {
        Self { context: value }
    }
}

impl GraphContextArenaRef {
    #[allow(clippy::mut_from_ref)]
    pub fn get_mut(&self) -> &mut GraphContextArena {
        unsafe { self.context.as_mut().unwrap() }
    }

    #[allow(clippy::mut_from_ref)]
    pub fn get_ref(&self) -> &GraphContextArena {
        unsafe { self.context.as_ref().unwrap() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        animation_graph::TimeUpdate,
        context::{graph_context::QueryOutputTime, node_states::StateKey},
        pose::{BonePose, Pose},
    };
    use bevy::{asset::Handle, math::Vec3, reflect::PartialReflect};

    #[derive(Clone, Debug, Reflect)]
    struct PrivateState {
        value: u64,
        #[reflect(ignore)]
        private_values: Vec<u64>,
    }

    #[test]
    fn replacing_a_child_graph_resets_only_its_subtree() {
        use bevy::asset::Assets;
        use crate::state_machine::high_level::StateId;
        let mut assets = Assets::<AnimationGraph>::default();
        let graphs = [0, 1, 2, 3].map(|_| assets.add(AnimationGraph::new()));
        let mut arena = GraphContextArena::new(graphs[0].id());
        let root = arena.get_toplevel_id();
        let node = NodeId::default();
        let key = SubContextId { ctx_id:root, node_id:node, state_id:None };
        let child = arena.get_sub_context_or_insert_default(key.clone(), graphs[1].id());
        let grandchild = arena.get_sub_context_or_insert_default(SubContextId { ctx_id:child, node_id:node, state_id:None }, graphs[1].id());
        let sibling = arena.get_sub_context_or_insert_default(SubContextId { ctx_id:root, node_id:node, state_id:Some(LowLevelStateId::HlState(StateId::default())) }, graphs[2].id());
        for context in [root, child, grandchild, sibling] {
            let state = arena.get_context_mut(context).unwrap();
            state.node_states.get_mut_or_insert_with(node, StateKey::Default, || PrivateState { value:7, private_values:vec![11] }).unwrap();
            state.node_states.set_time(node, StateKey::Default, 0.5);
            state.node_caches.set_output_data(node, StateKey::Default, "value".into(), 0.5_f32.into());
            state.node_caches.mark_update_started(node, StateKey::Default);
            state.node_caches.mark_updated(node, StateKey::Default);
            state.query_output_time = QueryOutputTime::Forced(TimeUpdate::Delta(0.25));
        }
        assert_eq!(arena.get_sub_context_or_insert_default(key.clone(), graphs[1].id()), child);
        assert_eq!(arena.get_context(child).unwrap().node_states.get::<PrivateState>(node, StateKey::Default).unwrap().value, 7);
        assert_eq!(arena.get_sub_context_or_insert_default(key.clone(), graphs[3].id()), child);
        assert_eq!(arena.get_context(child).unwrap().get_graph_id(), graphs[3].id());
        for context in [child, grandchild] {
            let state = arena.get_context(context).unwrap();
            assert!(state.node_states.get::<PrivateState>(node, StateKey::Default).is_err());
            assert!(!state.node_caches.is_updated(node, StateKey::Default));
            assert!(state.node_caches.get_output_data(node, StateKey::Default, "value".into()).is_none());
            assert!(matches!(state.query_output_time, QueryOutputTime::None));
        }
        for context in [root, sibling] {
            let state = arena.get_context(context).unwrap();
            assert_eq!(state.node_states.get::<PrivateState>(node, StateKey::Default).unwrap().value, 7);
            assert!(state.node_caches.is_updated(node, StateKey::Default));
        }
        for index in 0..16 {
            assert_eq!(arena.get_sub_context_or_insert_default(key.clone(), graphs[if index % 2 == 0 { 1 } else { 3 }].id()), child);
        }
        assert_eq!(arena.iter_context_ids().count(), 4);
    }

    #[test]
    fn snapshot_clone_preserves_hierarchy_caches_and_isolates_state() {
        let graph = Handle::<AnimationGraph>::default().id();
        let mut arena = GraphContextArena::new(graph);
        let node = NodeId::default();
        let child_key = SubContextId {
            ctx_id: arena.get_toplevel_id(),
            node_id: node,
            state_id: None,
        };
        let child = arena.get_sub_context_or_insert_default(child_key.clone(), graph);
        let temporary = StateKey::Temporary(uuid::Uuid::new_v4());
        let state = arena.get_context_mut(child).unwrap();
        state
            .node_states
            .get_mut_or_insert_with(node, StateKey::Default, || PrivateState {
                value: 7,
                private_values: vec![11],
            })
            .unwrap();
        state.node_states.set_time(node, StateKey::Default, 0.25);
        arena.next_frame();
        let state = arena.get_context_mut(child).unwrap();
        state
            .node_states
            .get_mut_or_insert_with::<PrivateState>(node, temporary, || unreachable!())
            .unwrap()
            .value = 9;
        state.node_states.set_time(node, temporary, 0.5);
        let mut pose = Pose {
            timestamp: 0.5,
            ..Default::default()
        };
        pose.add_bone(
            BonePose {
                translation: Some(Vec3::new(1.0, 2.0, 3.0)),
                ..Default::default()
            },
            Default::default(),
        );
        state
            .node_caches
            .set_output_data(node, temporary, "pose".into(), pose.into());
        state
            .node_caches
            .set_output_data(node, StateKey::Default, "weight".into(), 0.5_f32.into());
        state.node_caches.mark_update_started(node, temporary);
        state.node_caches.mark_updated(node, temporary);
        state.node_caches.set_duration(node, temporary, Some(1.0));
        state
            .node_caches
            .set_output_time_update(node, temporary, TimeUpdate::Delta(0.1));
        state.node_caches.set_input_time_update(
            node,
            temporary,
            "time".into(),
            TimeUpdate::Absolute(0.5),
        );
        state.query_output_time = QueryOutputTime::from_key(temporary, TimeUpdate::Absolute(0.5));
        let mut snapshot: GraphContextArena = arena.clone();
        let reflected = arena
            .reflect_clone()
            .unwrap()
            .take::<GraphContextArena>()
            .unwrap();
        for copy in [&snapshot, &reflected] {
            assert_eq!(copy.frame_index(), 1);
            assert_eq!(copy.hierarchy[&child_key], child);
            assert_eq!(copy.iter_context_ids().count(), 2);
            let state = copy.get_context(child).unwrap();
            assert_eq!(state.get_graph_id(), graph);
            assert_eq!(
                state
                    .node_states
                    .get::<PrivateState>(node, StateKey::Default)
                    .unwrap()
                    .value,
                7
            );
            assert_eq!(
                state
                    .node_states
                    .get::<PrivateState>(node, temporary)
                    .unwrap()
                    .value,
                9
            );
            assert_eq!(
                state
                    .node_states
                    .get::<PrivateState>(node, temporary)
                    .unwrap()
                    .private_values,
                [11]
            );
            assert_eq!(state.node_states.get_last_time(node), 0.25);
            assert_eq!(state.node_states.get_time(node, temporary), 0.5);
            assert!(state.node_caches.is_update_started(node, temporary));
            assert!(state.node_caches.is_updated(node, temporary));
            assert_eq!(
                state.node_caches.get_duration(node, temporary).unwrap(),
                Some(1.0)
            );
            assert!(
                matches!(state.node_caches.get_output_time_update(node, temporary), Ok(TimeUpdate::Delta(v)) if v == 0.1)
            );
            assert!(
                matches!(state.node_caches.get_input_time_update(node, temporary, "time".into()), Ok(TimeUpdate::Absolute(v)) if v == 0.5)
            );
            assert!(
                matches!(state.query_output_time.get(temporary), Some(TimeUpdate::Absolute(v)) if v == 0.5)
            );
            let pose = state
                .node_caches
                .get_output_data(node, temporary, "pose".into())
                .unwrap()
                .into_pose()
                .unwrap();
            assert_eq!(pose.timestamp, 0.5);
            assert_eq!(pose.bones[0].translation, Some(Vec3::new(1.0, 2.0, 3.0)));
            assert_eq!(
                state
                    .node_caches
                    .get_output_data(node, StateKey::Default, "weight".into())
                    .unwrap()
                    .as_f32()
                    .unwrap(),
                0.5
            );
        }
        snapshot.get_context_mut(child).unwrap().node_states.clear();
        snapshot.next_frame();
        let original = arena.get_context(child).unwrap();
        assert_eq!(
            original
                .node_states
                .get::<PrivateState>(node, temporary)
                .unwrap()
                .value,
            9
        );
        assert!(original.node_caches.is_updated(node, temporary));
        assert_eq!(arena.frame_index(), 1);
        assert_eq!(snapshot.frame_index(), 2);
        assert!(
            !snapshot
                .get_context(child)
                .unwrap()
                .node_caches
                .is_updated(node, temporary)
        );
    }
}
