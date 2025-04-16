use crate::die;
use async_channel::{Receiver, Sender, TrySendError};
use bevy_ecs::prelude::*;
use bevy_ecs::system::{BoxedSystem, IntoSystem, SystemId};
use bevy_platform::collections::{HashMap, HashSet};
use std::any::TypeId;
use std::marker::PhantomData;

#[derive(Default, Debug, Resource)]
pub struct WaiterCache(HashMap<TypeId, SystemId>);

#[derive(Default, Debug, Resource)]
pub struct ActiveWaiters(HashSet<SystemId>);

pub(crate) struct StartWaitingFor<CRE> {
	tx: Sender<CRE>,
	target: Option<Entity>,
	system: fn() -> BoxedSystem,
	name: Name,
}

impl<C: Component + Clone> StartWaitingFor<C> {
	fn component_system() -> BoxedSystem {
		let system = IntoSystem::into_system(process_waiting_components::<C>);
		Box::new(system)
	}

	pub(crate) fn component(target: Entity) -> (Self, Receiver<C>) {
		let (tx, rx) = async_channel::bounded(1);
		let command = Self {
			tx,
			target: Some(target),
			system: Self::component_system,
			name: Name::new("WaitingFor(Component)"),
		};
		(command, rx)
	}
}

impl<R: Resource + Clone> StartWaitingFor<R> {
	fn resource_system() -> BoxedSystem {
		let system = IntoSystem::into_system(process_waiting_resources::<R>);
		Box::new(system)
	}

	pub(crate) fn resource() -> (Self, Receiver<R>) {
		let (tx, rx) = async_channel::bounded(1);
		let command = Self {
			tx,
			target: None,
			system: Self::resource_system,
			name: Name::new("WaitingFor(Resource)"),
		};
		(command, rx)
	}
}

impl<E: Event + Clone> StartWaitingFor<E> {
	fn event_system() -> BoxedSystem {
		let system = IntoSystem::into_system(process_waiting_events::<E>);
		Box::new(system)
	}

	pub(crate) fn events() -> (Self, Receiver<E>) {
		let (tx, rx) = async_channel::unbounded();
		let command = Self {
			tx,
			target: None,
			system: Self::event_system,
			name: Name::new("WaitingFor(Events)"),
		};
		(command, rx)
	}
}

impl<CRE: Send + 'static> StartWaitingFor<CRE> {
	fn ensure(&self, cache: &mut WaiterCache, world: &mut World) -> SystemId {
		let type_id = TypeId::of::<CRE>();
		*cache.0.entry(type_id).or_insert_with(|| {
			let system = (self.system)();
			world.register_boxed_system(system)
		})
	}

	fn spawn_waiter(self, world: &mut World) {
		match self.target {
			None => world.spawn((self.name, WaitingFor(self.tx))),
			Some(id) => world.spawn((self.name, WaitingFor(self.tx), Target(id))),
		};
	}
}

impl<CRE: Send + 'static> Command for StartWaitingFor<CRE> {
	fn apply(self, world: &mut World) {
		let system_id = world.resource_scope(|world, mut cache| self.ensure(&mut cache, world));
		world.resource_mut::<ActiveWaiters>().0.insert(system_id);
		self.spawn_waiter(world);
		world.run_system(system_id).unwrap_or_else(die);
	}
}

struct StopWaitingFor<CRE>(PhantomData<CRE>);

impl<CRE> Default for StopWaitingFor<CRE> {
	fn default() -> Self {
		Self(PhantomData)
	}
}

impl<CRE: Send + 'static> Command for StopWaitingFor<CRE> {
	fn apply(self, world: &mut World) {
		let type_id = TypeId::of::<CRE>();
		let system_id = world.resource::<WaiterCache>().0.get(&type_id).copied();
		if let Some(system_id) = system_id {
			world.resource_mut::<ActiveWaiters>().0.remove(&system_id);
		}
	}
}

#[derive(Component)]
#[component(storage = "SparseSet")]
struct WaitingFor<CRE>(Sender<CRE>);

#[derive(Component)]
#[component(storage = "SparseSet")]
struct Target(Entity);

fn process_waiting_components<C: Component + Clone>(
	mut commands: Commands,
	query: Query<(Entity, &WaitingFor<C>, &Target)>,
	components: Query<&C>,
) {
	if query.is_empty() {
		commands.queue(StopWaitingFor::<C>::default());
		return;
	}

	for (id, waiting_for, target) in query.iter() {
		if let Ok(component) = components.get(target.0) {
			waiting_for
				.0
				.try_send(component.clone())
				.unwrap_or_else(die);
			commands.entity(id).despawn();
		}
	}
}

fn process_waiting_resources<R: Resource + Clone>(
	mut commands: Commands,
	query: Query<(Entity, &WaitingFor<R>)>,
	resource: Option<Res<R>>,
) {
	if query.is_empty() {
		commands.queue(StopWaitingFor::<R>::default());
		return;
	}

	for (id, waiting_for) in query.iter() {
		if let Some(resource) = &resource {
			waiting_for
				.0
				.try_send((*resource).clone())
				.unwrap_or_else(die);
			commands.entity(id).despawn();
		}
	}
}

fn process_waiting_events<E: Event + Clone>(
	mut commands: Commands,
	query: Query<(Entity, &WaitingFor<E>)>,
	mut event_reader: EventReader<E>,
) {
	if query.is_empty() {
		commands.queue(StopWaitingFor::<E>::default());
		return;
	}

	let events: Vec<&E> = event_reader.read().collect();
	if events.is_empty() {
		return;
	}

	for (id, waiting_for) in query.iter() {
		'events: for &event in &events {
			if let Err(e) = waiting_for.0.try_send(event.clone()) {
				match e {
					e @ TrySendError::Full(_) => die(e),
					TrySendError::Closed(_) => {
						commands.entity(id).despawn();
						break 'events;
					}
				}
			}
		}
	}
}

pub(crate) fn drive_waiting_for(mut commands: Commands, waiters: Res<ActiveWaiters>) {
	for system_id in &waiters.0 {
		commands.run_system(*system_id);
	}
}

pub(crate) fn initialize_waiters(mut commands: Commands) {
	commands.init_resource::<WaiterCache>();
	commands.init_resource::<ActiveWaiters>();
}

#[cfg(test)]
mod tests {
	use super::*;
	use bevy::diagnostic::FrameCount;
	use bevy::prelude::*;

	#[derive(Clone, Event)]
	struct MyEvent;

	#[test]
	fn smoke() {
		let mut app = App::new();
		app.add_plugins(MinimalPlugins)
			.init_resource::<WaiterCache>()
			.init_resource::<ActiveWaiters>()
			.add_event::<MyEvent>()
			.add_systems(Update, drive_waiting_for);

		let id = app.world_mut().spawn_empty().id();
		let (start_waiting_for, name_rx) = StartWaitingFor::<Name>::component(id);
		start_waiting_for.apply(app.world_mut());
		assert!(name_rx.try_recv().is_err());

		app.update();
		assert!(name_rx.try_recv().is_err());

		app.world_mut().entity_mut(id).insert(Name::new("Frank"));
		app.update();
		assert_eq!(name_rx.try_recv().unwrap(), Name::new("Frank"));

		let id = app.world_mut().spawn(Name::new("Tim")).id();
		let (start_waiting_for, name_rx) = StartWaitingFor::<Name>::component(id);
		start_waiting_for.apply(app.world_mut());
		assert_eq!(name_rx.try_recv().unwrap(), Name::new("Tim"));

		app.update();

		let (start_waiting_for, time_rx) = StartWaitingFor::<FrameCount>::resource();
		start_waiting_for.apply(app.world_mut());
		assert_eq!(time_rx.try_recv().unwrap().0, 3);

		app.update();

		let (start_waiting_for, events_rx) = StartWaitingFor::<MyEvent>::events();
		start_waiting_for.apply(app.world_mut());
		assert!(events_rx.try_recv().is_err());

		app.world_mut().send_event(MyEvent);
		app.world_mut().send_event(MyEvent);
		app.update();
		assert!(events_rx.try_recv().is_ok());
		assert!(events_rx.try_recv().is_ok());
		assert!(events_rx.try_recv().is_err());

		assert_eq!(
			app.world().get_resource::<ActiveWaiters>().unwrap().0.len(),
			1
		);
		assert_eq!(
			app.world().get_resource::<WaiterCache>().unwrap().0.len(),
			3
		);
	}
}
