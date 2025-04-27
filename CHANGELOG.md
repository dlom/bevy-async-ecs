# 0.8.1

- Added `AsyncWorld::run_system()`
- Added `AsyncWorld::trigger()`
- Added `AsyncWorld::trigger_targets()`
- Bug fixes (#11)

# 0.8.0

- Updated for bevy 0.16

# 0.7.0

- Updated for bevy 0.15

# 0.6.1

- Wasm support

# 0.6.0

- Updated for bevy 0.14

# 0.5.1

- Added `AsyncSystem::unregister()` and `AsyncIOSystem::unregister()`

# 0.5.0

- Updated for bevy 0.13

# 0.4.1

- `impl Clone` for `AsyncSystem` and `AsyncIOSystem`

# 0.4.0

- Properly exposed `CommandQueueSender` as a public API.
- Doc improvements

# 0.3.0

- (Mostly) complete rewrite of internal API.
- Removed dependence on `bevy_reflect`.
- `operation` module has been removed.
- `OperationSender` is now `CommandQueueSender`.
- Public API is... mostly unchanged!
- Unified component operations (`insert_component` and `insert_bundle` became `insert`, etc...).
- Event support (`send_event`, `wait_for_event`, etc...).

# 0.2.0

- `OperationSender` is now fully exposed as an internal API.

# 0.1.0

- Initial release.
