#[cfg(feature = "physics_avian")]
use avian3d::prelude::PhysicsSystems;
use bevy::{
    app::{App, Plugin, PreUpdate},
    asset::AssetApp,
    ecs::{
        intern::Interned,
        schedule::{IntoScheduleConfigs, ScheduleCleanupPolicy, ScheduleLabel, SystemSet},
    },
    reflect::prelude::ReflectDefault,
    transform::TransformSystems,
};

#[cfg(feature = "physics_avian")]
use crate::physics_systems_avian::{
    read_back_poses_avian, spawn_missing_ragdolls_avian, update_ragdoll_rigidbodies,
    update_ragdolls_avian, update_relative_kinematic_body_velocities,
    update_relative_kinematic_position_based_body_velocities,
};
use crate::{
    animated_scene::{
        AnimatedScene, loader::AnimatedSceneLoader, locate_animated_scene_player,
        spawn_animated_scenes,
    },
    animation_clip::{EntityPath, GraphClip, Interpolation, loader::GraphClipLoader},
    animation_graph::{AnimationGraph, loader::AnimationGraphLoader},
    animation_graph_player::AnimationGraphPlayer,
    animation_node::AnimationNode,
    edge_data::{
        DataSpec, DataValue,
        bone_mask::BoneMask,
        events::{AnimationEvent, EventQueue, SampledEvent},
    },
    pose::{Pose, RootMotionDelta, RootMotionMode},
    ragdoll::{
        bone_mapping::RagdollBoneMap, bone_mapping_loader::RagdollBoneMapLoader,
        definition::Ragdoll, definition_loader::RagdollLoader,
    },
    skeleton::{Skeleton, loader::SkeletonLoader},
    state_machine::high_level::{StateMachine, loader::StateMachineLoader},
    symmetry::{config::SymmetryConfig, serial::SymmetryConfigSerial},
    systems::{
        RootMotionOutput, animation_player, animation_player_deferred_gizmos,
        apply_animation_to_targets, extract_root_motion,
    },
};

/// Adds animation support to an app
pub struct AnimationGraphCorePlugin {
    pub physics_schedule: Interned<dyn ScheduleLabel>,
    pub final_schedule: Interned<dyn ScheduleLabel>,
}

/// Installs a host's graph sampling and presentation systems at startup.
/// The physics callback receives the schedule driving the solver. The pose callback
/// receives the display schedule. Adding this first skips the standard playback systems.
#[derive(Clone, Copy)]
pub struct AnimationGraphPlayback {
    pub physics: fn(&mut App, Interned<dyn ScheduleLabel>),
    pub pose: fn(&mut App, Interned<dyn ScheduleLabel>),
}

/// Ordered integration points for graph players, ragdoll drivers and visible skeletons.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, SystemSet)]
pub enum AnimationPlaybackSystems {
    /// Sample animation after ragdoll spawning and before updating bodies.
    Sample,
    /// Write position-based kinematic velocities from the new ragdoll targets.
    KinematicTargets,
    /// Apply the display pose before drawing animation gizmos.
    WritePose,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, SystemSet)]
struct DefaultPlayback;

impl Plugin for AnimationGraphPlayback {
    fn build(&self, _app: &mut App) {}

    fn finish(&self, app: &mut App) {
        // A host may install its playback after a dependency already installed the standard plugin.
        let Some((physics, pose)) = app.get_added_plugins::<AnimationGraphCorePlugin>()
            .first()
            .map(|core| (core.physics_schedule, core.final_schedule)) else { return };
        if !app.get_schedule(physics).is_some_and(|s| s.graph().system_sets.contains(DefaultPlayback)) {
            return; // The core installed this host's callbacks during build.
        }
        // Bevy validates before removal. Finish removing the old writers before adding any replacements.
        app.remove_systems_in_set(physics, DefaultPlayback, ScheduleCleanupPolicy::RemoveSetAndSystems)
            .expect("registered graph sampling schedule");
        if pose != physics {
            app.remove_systems_in_set(pose, DefaultPlayback, ScheduleCleanupPolicy::RemoveSetAndSystems)
                .expect("registered graph presentation schedule");
        }
        (self.physics)(app, physics);
        (self.pose)(app, pose);
    }
}

#[derive(Clone, Debug, Copy, PartialEq, Eq, Hash, SystemSet)]
pub enum AnimationGraphSet {
    /// This set runs in the same schedule as the physics update, before the physics update
    PrePhysics,
    /// This set runs in the same schedule as the physics update, after the physics update
    PostPhysics,
    /// This set runs at the end of the GPU frame (i.e. generally PostUpdate), it does not depend
    /// on physics update schedule.
    Final,
}

impl Plugin for AnimationGraphCorePlugin {
    fn build(&self, app: &mut App) {
        self.register_assets(app);
        self.register_types(app);
        self.register_component_hooks(app);

        app.configure_sets(
            self.physics_schedule,
            (
                AnimationGraphSet::PrePhysics,
                AnimationGraphSet::PostPhysics,
                AnimationGraphSet::Final,
            )
                .chain(),
        );

        app.configure_sets(
            self.physics_schedule,
            AnimationGraphSet::Final.before(TransformSystems::Propagate),
        );

        #[cfg(feature = "physics_avian")]
        {
            app.configure_sets(
                self.physics_schedule,
                (
                    AnimationGraphSet::PrePhysics.before(PhysicsSystems::First),
                    AnimationGraphSet::PostPhysics.after(PhysicsSystems::Last),
                ),
            );

            self.register_physics_types(app);
        }

        app.add_systems(PreUpdate, spawn_animated_scenes);
        app.configure_sets(
            self.physics_schedule,
            (AnimationPlaybackSystems::Sample, AnimationPlaybackSystems::KinematicTargets)
                .chain()
                .in_set(AnimationGraphSet::PrePhysics),
        ).configure_sets(
            self.final_schedule,
            AnimationPlaybackSystems::WritePose.in_set(AnimationGraphSet::Final),
        );

        #[cfg(feature = "physics_avian")]
        app.add_systems(
            self.physics_schedule,
            (
                spawn_missing_ragdolls_avian.before(AnimationPlaybackSystems::Sample),
                (update_ragdoll_rigidbodies, update_ragdolls_avian).chain()
                    .after(AnimationPlaybackSystems::Sample)
                    .before(AnimationPlaybackSystems::KinematicTargets),
                update_relative_kinematic_body_velocities.after(AnimationPlaybackSystems::KinematicTargets),
            ).in_set(AnimationGraphSet::PrePhysics),
        );

        app.add_systems(
            self.final_schedule,
            animation_player_deferred_gizmos.after(AnimationPlaybackSystems::WritePose).in_set(AnimationGraphSet::Final),
        );

        if let Some(playback) = app.get_added_plugins::<AnimationGraphPlayback>().into_iter().copied().next() {
            (playback.physics)(app, self.physics_schedule);
            (playback.pose)(app, self.final_schedule);
        } else {
            app.add_systems(
                self.physics_schedule,
                (animation_player, extract_root_motion).chain()
                    .in_set(AnimationPlaybackSystems::Sample)
                    .in_set(DefaultPlayback),
            );
            #[cfg(feature = "physics_avian")]
            app.add_systems(
                self.physics_schedule,
                (
                    update_relative_kinematic_position_based_body_velocities.in_set(AnimationPlaybackSystems::KinematicTargets),
                    read_back_poses_avian.in_set(AnimationGraphSet::PostPhysics),
                ).in_set(DefaultPlayback),
            );
            app.add_systems(
                self.final_schedule,
                apply_animation_to_targets.in_set(AnimationPlaybackSystems::WritePose).in_set(DefaultPlayback),
            );
        }

        app.add_observer(locate_animated_scene_player);
    }
}

impl AnimationGraphCorePlugin {
    /// Registers asset types and their loaders
    fn register_assets(&self, app: &mut App) {
        app.init_asset::<GraphClip>()
            .init_asset_loader::<GraphClipLoader>()
            .register_asset_reflect::<GraphClip>();
        app.init_asset::<AnimationGraph>()
            .init_asset_loader::<AnimationGraphLoader>()
            .register_asset_reflect::<AnimationGraph>();
        app.init_asset::<AnimatedScene>()
            .init_asset_loader::<AnimatedSceneLoader>()
            .register_asset_reflect::<AnimatedScene>();
        app.init_asset::<StateMachine>()
            .init_asset_loader::<StateMachineLoader>()
            .register_asset_reflect::<StateMachine>();
        app.init_asset::<Skeleton>()
            .init_asset_loader::<SkeletonLoader>()
            .register_asset_reflect::<Skeleton>();
        app.init_asset::<Ragdoll>()
            .init_asset_loader::<RagdollLoader>()
            .register_asset_reflect::<Ragdoll>();
        app.init_asset::<RagdollBoneMap>()
            .init_asset_loader::<RagdollBoneMapLoader>()
            .register_asset_reflect::<RagdollBoneMap>();
    }

    /// "Other" reflect registrations
    fn register_types(&self, app: &mut App) {
        app //
            .register_type::<Interpolation>()
            .register_type::<AnimationGraphPlayer>()
            .register_type::<EntityPath>()
            .register_type::<BoneMask>()
            .register_type::<Pose>()
            .register_type::<AnimationEvent>()
            .register_type::<SampledEvent>()
            .register_type::<EventQueue>()
            .register_type::<AnimationEvent>()
            .register_type::<SampledEvent>()
            .register_type::<DataValue>()
            .register_type::<DataSpec>()
            .register_type::<AnimationNode>()
            .register_type::<SymmetryConfig>()
            .register_type::<SymmetryConfigSerial>()
            .register_type::<RootMotionDelta>()
            .register_type::<RootMotionMode>()
            .register_type::<RootMotionOutput>()
            .register_type::<()>()
            .register_type_data::<(), ReflectDefault>();
    }

    #[cfg(feature = "physics_avian")]
    fn register_physics_types(&self, app: &mut App) {
        use crate::ragdoll::relative_kinematic_body::{
            RelativeKinematicBody, RelativeKinematicBodyPositionBased,
        };

        app.register_type::<RelativeKinematicBody>();
        app.register_type::<RelativeKinematicBodyPositionBased>();
    }

    fn register_component_hooks(&self, app: &mut App) {
        app.world_mut()
            .register_component_hooks::<AnimationGraphPlayer>()
            .on_discard(|mut world, context| {
                if let Some(spawned_ragdoll) = world
                    .entity(context.entity)
                    .get::<AnimationGraphPlayer>()
                    .and_then(|p| p.spawned_ragdoll.as_ref())
                    .map(|s| s.root)
                {
                    world.commands().entity(spawned_ragdoll).despawn();
                }
            });
    }
}
