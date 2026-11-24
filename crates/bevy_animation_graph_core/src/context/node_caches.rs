use std::sync::Arc;

use bevy::{
    platform::collections::{Equivalent, HashMap, HashSet},
    reflect::Reflect,
};

use crate::{
    animation_graph::{NodeId, PinId, SourcePin, TargetPin, TimeUpdate},
    context::node_states::StateKey,
    duration_data::DurationData,
    edge_data::DataValue,
    errors::GraphError,
};

#[derive(Reflect, Clone, Default, Debug)]
pub struct NodeCache {
    pub output_data: HashMap<(StateKey, PinId), DataValue>,
    /// Time update coming from the "output time" pin. Perhaps should be called "input time
    /// update".
    pub output_time_update: HashMap<StateKey, TimeUpdate>,
    /// Time updates sent back to nodes via "input time" pins
    pub input_time_updates: HashMap<(StateKey, PinId), TimeUpdate>,
    pub duration: HashMap<StateKey, DurationData>,
    /// Whether the node update is started
    pub update_started: HashSet<StateKey>,
    /// Whether the node update is completed
    pub updated: HashSet<StateKey>,
}

#[derive(Reflect, Clone, Default, Debug)]
pub struct NodeCaches {
    caches: HashMap<NodeId, Arc<NodeCache>>,
}

#[derive(Hash)]
struct PinLookup<'a>(StateKey, &'a str);

impl Equivalent<(StateKey, PinId)> for PinLookup<'_> {
    fn equivalent(&self, other: &(StateKey, PinId)) -> bool {
        self.0 == other.0 && self.1 == other.1.as_str()
    }
}

impl NodeCaches {
    pub fn next_frame(&mut self) {
        self.caches.clear();
    }

    pub fn get_duration(&self, node_id: NodeId, key: StateKey) -> Result<DurationData, GraphError> {
        let error = || GraphError::DurationMissing(SourcePin::NodeTime(node_id));

        self.caches
            .get(&node_id)
            .ok_or_else(error)
            .and_then(|c| c.duration.get(&key).ok_or_else(error))
            .cloned()
    }

    pub fn set_duration(&mut self, node_id: NodeId, key: StateKey, duration: DurationData) {
        self.cache_mut(node_id).duration.insert(key, duration);
    }

    pub fn get_output_data(&self, node_id: NodeId, key: StateKey, pin: PinId) -> Option<DataValue> {
        self.get_output_data_ref(node_id, key, &pin).cloned()
    }

    /// Borrows a node's cached output without copying its key or value.
    pub fn get_output_data_ref(
        &self,
        node_id: NodeId,
        key: StateKey,
        pin: &str,
    ) -> Option<&DataValue> {
        self.caches
            .get(&node_id)
            .and_then(|c| c.output_data.get(&PinLookup(key, pin)))
    }

    pub fn set_output_data(&mut self, node_id: NodeId, key: StateKey, pin: PinId, data: DataValue) {
        self.cache_mut(node_id).output_data.insert((key, pin), data);
    }

    pub fn get_output_time_update(
        &self,
        node_id: NodeId,
        key: StateKey,
    ) -> Result<TimeUpdate, GraphError> {
        let error = || GraphError::TimeUpdateMissingFwd(SourcePin::NodeTime(node_id));

        self.caches
            .get(&node_id)
            .ok_or_else(error)
            .and_then(|c| c.output_time_update.get(&key).ok_or_else(error))
            .cloned()
    }

    pub fn set_output_time_update(&mut self, node_id: NodeId, key: StateKey, update: TimeUpdate) {
        self.cache_mut(node_id)
            .output_time_update
            .insert(key, update);
    }

    pub fn get_input_time_update(
        &self,
        node_id: NodeId,
        key: StateKey,
        pin: PinId,
    ) -> Result<TimeUpdate, GraphError> {
        self.get_input_time_update_by_pin(node_id, key, &pin)
    }

    pub(crate) fn get_input_time_update_by_pin(
        &self,
        node_id: NodeId,
        key: StateKey,
        pin: &str,
    ) -> Result<TimeUpdate, GraphError> {
        self.caches
            .get(&node_id)
            .and_then(|c| c.input_time_updates.get(&PinLookup(key, pin)))
            .cloned()
            .ok_or_else(|| GraphError::TimeUpdateMissingBack(TargetPin::NodeTime(node_id, pin.into())))
    }

    pub fn set_input_time_update(
        &mut self,
        node_id: NodeId,
        key: StateKey,
        pin: PinId,
        update: TimeUpdate,
    ) {
        self.cache_mut(node_id)
            .input_time_updates
            .insert((key, pin), update);
    }

    pub fn is_updated(&self, node_id: NodeId, key: StateKey) -> bool {
        self.caches
            .get(&node_id)
            .is_some_and(|c| c.updated.contains(&key))
    }

    pub fn mark_updated(&mut self, node_id: NodeId, key: StateKey) {
        self.cache_mut(node_id).updated.insert(key);
    }

    pub fn is_update_started(&self, node_id: NodeId, key: StateKey) -> bool {
        self.caches
            .get(&node_id)
            .is_some_and(|c| c.update_started.contains(&key))
    }

    pub fn mark_update_started(&mut self, node_id: NodeId, key: StateKey) {
        self.cache_mut(node_id).update_started.insert(key);
    }

    fn cache_mut(&mut self, node_id: NodeId) -> &mut NodeCache {
        Arc::make_mut(self.caches.entry(node_id).or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pose::{BonePose, Pose};

    #[test]
    fn borrowed_pin_keys_distinguish_states_and_pins() {
        let node = NodeId::default();
        let key = StateKey::Temporary(uuid::Uuid::nil());
        let mut caches = NodeCaches::default();
        caches.set_output_data(node, key, "pose".into(), DataValue::F32(2.0));
        caches.set_input_time_update(node, key, "pose".into(), TimeUpdate::Delta(0.25));

        assert_eq!(
            caches.get_output_data_ref(node, key, "pose"),
            Some(&DataValue::F32(2.0))
        );
        assert!(caches.get_output_data_ref(node, key, "phase").is_none());
        assert!(caches.get_output_data_ref(node, StateKey::Default, "pose").is_none());
        assert!(matches!(
            caches.get_input_time_update_by_pin(node, key, "pose"),
            Ok(TimeUpdate::Delta(0.25))
        ));
        assert!(matches!(
            caches.get_input_time_update_by_pin(node, key, "phase"),
            Err(GraphError::TimeUpdateMissingBack(TargetPin::NodeTime(id, pin)))
                if id == node && pin == "phase"
        ));
    }

    #[test]
    fn snapshot_shares_cached_pose_until_the_node_changes() {
        let node = NodeId::default();
        let key = StateKey::Default;
        let pin = "pose".to_owned();
        let mut pose = Pose::default();
        pose.add_bone(BonePose::default(), Default::default());
        let mut caches = NodeCaches::default();
        caches.set_output_data(node, key, pin.clone(), pose.into());
        let mut snapshot = caches.clone();
        let pose_values = |caches: &NodeCaches| {
            let DataValue::Pose(pose) = &caches.caches[&node].output_data[&(key, pin.clone())]
            else {
                panic!("cached value is not a pose");
            };
            pose.bones.as_ptr()
        };
        assert_eq!(
            pose_values(&snapshot),
            pose_values(&caches),
            "snapshot copied unread pose"
        );
        snapshot.set_duration(node, key, Some(2.0));
        assert_ne!(pose_values(&snapshot), pose_values(&caches));
        assert!(caches.get_duration(node, key).is_err());
        assert_eq!(snapshot.get_duration(node, key).unwrap(), Some(2.0));
        snapshot.set_output_data(
            node,
            key,
            pin.clone(),
            Pose {
                timestamp: 3.0,
                ..Default::default()
            }
            .into(),
        );
        assert_eq!(
            snapshot
                .get_output_data(node, key, pin.clone())
                .unwrap()
                .into_pose()
                .unwrap()
                .timestamp,
            3.0
        );
        assert_eq!(
            caches
                .get_output_data(node, key, pin)
                .unwrap()
                .into_pose()
                .unwrap()
                .timestamp,
            0.0
        );
        caches.next_frame();
        assert_eq!(snapshot.get_duration(node, key).unwrap(), Some(2.0));
    }
}
