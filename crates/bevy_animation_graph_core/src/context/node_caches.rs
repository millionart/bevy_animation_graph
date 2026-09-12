use bevy::{
    platform::collections::{HashMap, HashSet},
    reflect::Reflect,
};

use crate::{
    animation_graph::{NodeId, PinId, SourcePin, TargetPin, TimeUpdate},
    context::node_states::StateKey,
    duration_data::DurationData,
    edge_data::DataValue,
    errors::GraphError,
};

#[derive(Reflect, Default, Debug)]
pub struct NodeCache {
    #[reflect(clone)]
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

#[derive(Reflect, Default, Debug)]
pub struct NodeCaches {
    caches: HashMap<NodeId, NodeCache>,
}

impl NodeCaches {
    pub fn next_frame(&mut self) {
        self.caches.clear();
    }

    pub fn get_duration(&self, node_id: NodeId, key: StateKey) -> Result<DurationData, GraphError> {
        let error = || GraphError::DurationMissing(SourcePin::NodeTime(node_id));

        self.caches
            .get(&node_id)
            .ok_or_else(&error)
            .and_then(|c| c.duration.get(&key).ok_or_else(&error))
            .cloned()
    }

    pub fn set_duration(&mut self, node_id: NodeId, key: StateKey, duration: DurationData) {
        self.cache_mut(node_id).duration.insert(key, duration);
    }

    pub fn get_output_data(&self, node_id: NodeId, key: StateKey, pin: PinId) -> Option<DataValue> {
        self.caches
            .get(&node_id)
            .and_then(|c| c.output_data.get(&(key, pin.clone())))
            .cloned()
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
            .ok_or_else(&error)
            .and_then(|c| c.output_time_update.get(&key).ok_or_else(&error))
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
        let error = || GraphError::TimeUpdateMissingBack(TargetPin::NodeTime(node_id, pin.clone()));

        self.caches
            .get(&node_id)
            .ok_or_else(&error)
            .and_then(|c| {
                c.input_time_updates
                    .get(&(key, pin.clone()))
                    .ok_or_else(&error)
            })
            .cloned()
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
        self.caches.entry(node_id).or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pose::{BonePose, Pose};
    use bevy::{
        math::{Quat, Vec3},
        reflect::PartialReflect,
    };

    #[test]
    fn reflection_snapshot_clones_cached_pose_independently() {
        let mut cache = NodeCaches::default();
        let node = NodeId::default();
        let key = StateKey::Temporary(uuid::Uuid::new_v4());
        let bone = BonePose {
            translation: Some(Vec3::new(1.0, 2.0, 3.0)),
            rotation: Some(Quat::from_rotation_y(0.5)),
            scale: Some(Vec3::splat(2.0)),
            ..Default::default()
        };
        let mut pose = Pose {
            timestamp: 0.25,
            ..Default::default()
        };
        pose.add_bone(bone.clone(), Default::default());
        cache.set_output_data(node, key, "pose".into(), pose.into());
        cache.set_output_data(node, StateKey::Default, "weight".into(), 0.5_f32.into());
        let snapshot = cache.reflect_clone().unwrap().take::<NodeCaches>().unwrap();
        cache.next_frame();
        assert!(cache.get_output_data(node, key, "pose".into()).is_none());
        let copied = snapshot
            .get_output_data(node, key, "pose".into())
            .unwrap()
            .into_pose()
            .unwrap();
        assert_eq!(copied.timestamp, 0.25);
        assert_eq!(copied.paths[&crate::id::BoneId::default()], 0);
        assert_eq!(copied.bones[0].translation, bone.translation);
        assert_eq!(copied.bones[0].rotation, bone.rotation);
        assert_eq!(copied.bones[0].scale, bone.scale);
        assert_eq!(
            snapshot
                .get_output_data(node, StateKey::Default, "weight".into())
                .unwrap()
                .as_f32()
                .unwrap(),
            0.5
        );
    }
}
