use bevy_ecs::prelude::*;
use bevy_ecs::system::Command;

pub(crate) fn insert<B: Bundle>(id: Entity, bundle: B) -> impl Command {
	move |world: &mut World| {
		world.entity_mut(id).insert(bundle);
	}
}

pub(crate) fn remove<B: Bundle>(id: Entity) -> impl Command {
	move |world: &mut World| {
		world.entity_mut(id).remove::<B>();
	}
}

pub(crate) fn insert_resource<R: Resource>(resource: R) -> impl Command {
	move |world: &mut World| {
		world.insert_resource(resource);
	}
}

pub(crate) fn remove_resource<R: Resource>() -> impl Command {
	move |world: &mut World| {
		world.remove_resource::<R>();
	}
}
