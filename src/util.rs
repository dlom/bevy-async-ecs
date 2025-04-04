use bevy_ecs::prelude::*;
use bevy_ecs::system::SystemId;

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

pub(crate) fn despawn(id: Entity) -> impl Command {
	move |world: &mut World| {
		world.despawn(id);
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

pub(crate) fn remove_system<I: SystemInput + 'static, O: 'static>(
	system_id: SystemId<I, O>,
) -> impl Command {
	move |world: &mut World| {
		let _ = world.unregister_system(system_id);
	}
}
