# Skid Monitor Client .NET Bindings

Client-side C# extension decisions are tracked in
[`skid-monitor-client/docs/rfcs/0002-csharp-extension-developer-experience.md`](../../docs/rfcs/0002-csharp-extension-developer-experience.md).

The `skid-monitor-client` .NET binding can stream received signals to an out-of-process .NET
extension host. The Rust client owns the TCP protocol and rendering path; C#
extensions receive newline-delimited JSON events over stdin.

Unity 6 uses .NET Standard 2.1 as its default API compatibility level, so the
extension SDK and sample extension target `netstandard2.1`. The out-of-process
extension host remains `net8.0` because it is an executable sidecar, not a Unity
managed plug-in.

## Projects

- `Skid.Monitor.Client.Sdk`: extension interface and signal context types.
- `Skid.Monitor.Client.ExtensionHost`: stdin event loop and assembly loader.
- `examples/Skid.Monitor.Client.SampleExtension`: minimal extension that counts signal events.

## Runtime Model

Runtime is split into three parts.

- `skid-monitor-client` is the supervisor and signal receiver. It owns TCP, rendering, process startup, and event delivery.
- `Skid.Monitor.Client.ExtensionHost` is the .NET runtime sidecar. The Rust client starts it through `SKID_MONITOR_EXTENSION_HOST` and writes newline-delimited JSON events to its stdin.
- `Skid.Monitor.Client.Sdk` is a separate library for extension authors. Extensions reference the SDK at build time and are loaded by the host at runtime.

Development mode can use `dotnet run --project ...ExtensionHost.csproj`. Packaged mode should publish the host as a framework-dependent or self-contained sidecar and point `SKID_MONITOR_EXTENSION_HOST` at that executable. Extension assemblies are listed with `SKID_MONITOR_DOTNET_EXTENSIONS` using the platform path separator.

The .NET host is intentionally out-of-process: a crashing extension should not take down the Rust client, and the Rust protocol layer does not need to embed CLR. Future runtime permissions should live in an extension manifest rather than in the SDK type alone.

## Run

Build the host and sample extension:

```sh
dotnet build client/skid-monitor-client/bindings/dotnet/Skid.Monitor.Client.ExtensionHost/Skid.Monitor.Client.ExtensionHost.csproj
dotnet build client/skid-monitor-client/bindings/dotnet/examples/Skid.Monitor.Client.SampleExtension/Skid.Monitor.Client.SampleExtension.csproj
```

Start the client with the .NET host:

```sh
SKID_MONITOR_DOTNET_EXTENSIONS=./client/skid-monitor-client/bindings/dotnet/examples/Skid.Monitor.Client.SampleExtension/bin/Debug/netstandard2.1/Skid.Monitor.Client.SampleExtension.dll \
SKID_MONITOR_EXTENSION_HOST="dotnet run --project client/skid-monitor-client/bindings/dotnet/Skid.Monitor.Client.ExtensionHost/Skid.Monitor.Client.ExtensionHost.csproj" \
cargo run -p skid-monitor-client
```

Each event has this envelope:

```json
{
  "schema": "skid.monitor.extension.v1",
  "type": "signal",
  "signal": {}
}
```
