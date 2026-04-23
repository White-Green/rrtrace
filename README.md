# rrtrace

`rrtrace` is a Ruby tracing tool with a Rust-based visualizer.
It records Ruby method calls, returns, GC transitions, and thread lifecycle events, then streams them to a separate GPU renderer process for real-time visualization.

## Features

- Trace Ruby `call` / `return` and C-level `c_call` / `c_return` events
- Capture GC enter / exit events
- Capture thread start / ready / suspend / resume / exit events
- Stream events through shared memory to a separate visualizer process
- Render the trace with a native Rust application built on `wgpu` and `winit`

## How It Works

`rrtrace` consists of two parts:

1. A Ruby native extension written in C
   - Installs Ruby tracepoints and internal thread event hooks
   - Writes trace events into a shared-memory ring buffer
   - Launches the visualizer process
2. A Rust visualizer
   - Reads events from shared memory
   - Builds trace state in background threads
   - Renders the result in a desktop window using the GPU

At a high level, the data flow is:

`Ruby VM` -> `C extension hooks` -> `shared-memory ring buffer` -> `Rust visualizer` -> `window`

## Usage

### CLI

Run a Ruby script under `rrtrace`:

```bash
rrtrace path/to/script.rb
```

You can also use the repository executable directly during development:

```bash
bundle exec exe/rrtrace path/to/script.rb
```

The command starts the visualizer process, opens a window, loads the target Ruby file, and stops tracing when the program exits.

### Ruby API

For manual control:

```ruby
require "rrtrace"

Rrtrace.start

# your Ruby code here

Rrtrace.stop
```

Available methods:

- `Rrtrace.start`
- `Rrtrace.stop`
- `Rrtrace.started?`
- `Rrtrace.visualizer_path`

`Rrtrace.stop` is also registered with `at_exit`, so tracing is cleaned up automatically when the process exits normally.

## Development

Install dependencies:

```bash
bin/setup
```

Build the extension and visualizer:

```bash
bundle exec rake build
```

Useful commands:

```bash
bundle exec rake compile
bundle exec rake standard
cargo test --locked
cargo fmt --all
cargo clippy --tests -- -D warnings
```

The Rust visualizer built during extension compilation is copied to `libexec/rrtrace` (or `libexec/rrtrace.exe` on Windows).

## License

This project is licensed under the MIT License. See [LICENSE.txt](LICENSE.txt).
