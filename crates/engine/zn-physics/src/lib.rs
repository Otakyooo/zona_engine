//! zn-physics: обёртки над rapier3d, character controller, raycast.

use glam::Vec3;
use nalgebra as na;
pub use rapier3d::prelude::RigidBodyHandle;
use rapier3d::prelude::*;

pub struct PhysicsWorld {
    pub gravity: na::Vector3<f32>,
    pub rigid_body_set: RigidBodySet,
    pub collider_set: ColliderSet,
    pub integration_params: IntegrationParameters,
    pub physics_pipeline: PhysicsPipeline,
    pub island_manager: IslandManager,
    pub broad_phase: DefaultBroadPhase,
    pub narrow_phase: NarrowPhase,
    pub impulse_joint_set: ImpulseJointSet,
    pub multibody_joint_set: MultibodyJointSet,
    pub ccd_solver: CCDSolver,
    pub query_pipeline: QueryPipeline,
}

impl PhysicsWorld {
    pub fn new() -> Self {
        Self {
            gravity: na::vector![0.0, -9.81, 0.0],
            rigid_body_set: RigidBodySet::new(),
            collider_set: ColliderSet::new(),
            integration_params: IntegrationParameters::default(),
            physics_pipeline: PhysicsPipeline::new(),
            island_manager: IslandManager::new(),
            broad_phase: DefaultBroadPhase::new(),
            narrow_phase: NarrowPhase::new(),
            impulse_joint_set: ImpulseJointSet::new(),
            multibody_joint_set: MultibodyJointSet::new(),
            ccd_solver: CCDSolver::new(),
            query_pipeline: QueryPipeline::new(),
        }
    }

    pub fn step(&mut self, dt: f32) {
        self.integration_params.dt = dt;
        let physics_hooks = ();
        let event_handler = ();
        self.physics_pipeline.step(
            &self.gravity,
            &self.integration_params,
            &mut self.island_manager,
            &mut self.broad_phase,
            &mut self.narrow_phase,
            &mut self.rigid_body_set,
            &mut self.collider_set,
            &mut self.impulse_joint_set,
            &mut self.multibody_joint_set,
            &mut self.ccd_solver,
            Some(&mut self.query_pipeline),
            &physics_hooks,
            &event_handler,
        );
    }

    pub fn add_static_box(&mut self, position: Vec3, half_extents: Vec3) {
        let rb = RigidBodyBuilder::fixed()
            .translation(na::vector![position.x, position.y, position.z])
            .build();
        let handle = self.rigid_body_set.insert(rb);
        let collider =
            ColliderBuilder::cuboid(half_extents.x, half_extents.y, half_extents.z).build();
        self.collider_set
            .insert_with_parent(collider, handle, &mut self.rigid_body_set);
    }

    pub fn add_character(&mut self, position: Vec3) -> RigidBodyHandle {
        let rb = RigidBodyBuilder::dynamic()
            .translation(na::vector![position.x, position.y, position.z])
            .lock_rotations()
            .build();
        let handle = self.rigid_body_set.insert(rb);
        let collider = ColliderBuilder::capsule_y(0.5, 0.3).build();
        self.collider_set
            .insert_with_parent(collider, handle, &mut self.rigid_body_set);
        handle
    }

    pub fn set_character_velocity(&mut self, handle: RigidBodyHandle, velocity: Vec3) {
        if let Some(rb) = self.rigid_body_set.get_mut(handle) {
            rb.set_linvel(na::vector![velocity.x, velocity.y, velocity.z], true);
        }
    }

    pub fn character_position(&self, handle: RigidBodyHandle) -> Vec3 {
        if let Some(rb) = self.rigid_body_set.get(handle) {
            let t = rb.translation();
            Vec3::new(t.x, t.y, t.z)
        } else {
            Vec3::ZERO
        }
    }
}

impl Default for PhysicsWorld {
    fn default() -> Self {
        Self::new()
    }
}
