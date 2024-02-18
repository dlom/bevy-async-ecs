use bevy::prelude::*;
use bevy::tasks::AsyncComputeTaskPool;
use bevy_async_ecs::*;
use key_wait::*;

fn main() {
	App::new()
		.add_plugins((DefaultPlugins, AsyncEcsPlugin, KeyWaitPlugin))
		.add_systems(Startup, |world: &mut World| {
			let async_world = AsyncWorld::from_world(world);
			let fut = async move {
				info!("started");
				let key_waiter = KeyWaiter::new(&async_world).await;
				info!("press the space bar...");
				loop {
					key_waiter.wait_for(KeyCode::Space).await;
					info!("you pressed the space bar!");
				}
			};
			AsyncComputeTaskPool::get().spawn(fut).detach();
		})
		.run();
}

mod key_wait {
	use bevy::prelude::*;
	use bevy_async_ecs::*;

	pub struct KeyWaiter(AsyncEntity);

	impl KeyWaiter {
		pub async fn new(world: &AsyncWorld) -> Self {
			let entity = world.spawn_named("KeyWaiter").await;
			Self(entity)
		}

		pub async fn wait_for(&self, key_code: KeyCode) {
			let _: KeyPressed = self.0.insert_wait_remove(WaitingForKey(key_code)).await;
		}
	}

	pub struct KeyWaitPlugin;

	impl Plugin for KeyWaitPlugin {
		fn build(&self, app: &mut App) {
			app.add_systems(
				PreUpdate,
				wait_for_key.run_if(any_with_component::<WaitingForKey>),
			);
		}
	}

	#[derive(Component)]
	pub struct WaitingForKey(pub KeyCode);

	impl Default for WaitingForKey {
		fn default() -> Self {
			Self(KeyCode::Space)
		}
	}

	#[derive(Default, Clone, Component)]
	pub struct KeyPressed;

	fn wait_for_key(
		mut commands: Commands,
		input: Res<ButtonInput<KeyCode>>,
		query: Query<(Entity, &WaitingForKey), Without<KeyPressed>>,
	) {
		for (id, wfk) in query.iter() {
			if input.just_pressed(wfk.0) {
				commands
					.entity(id)
					.remove::<WaitingForKey>()
					.insert(KeyPressed);
			}
		}
	}
}
