# Skid Monitor Client .NET Bindings

The `skid-monitor-client` .NET binding can stream received signals to an out-of-process .NET
extension host. The Rust client owns the TCP protocol and rendering path; C#
extensions receive newline-delimited JSON events over stdin.

## Projects

- `Skid.Monitor.Client.Sdk`: extension interface and signal context types.
- `Skid.Monitor.Client.ExtensionHost`: stdin event loop and assembly loader.
- `examples/Skid.Monitor.Client.SampleExtension`: minimal extension that counts signal events.

## Run

Build the host and sample extension:

```sh
dotnet build skid-monitor-client/bindings/dotnet/Skid.Monitor.Client.ExtensionHost/Skid.Monitor.Client.ExtensionHost.csproj
dotnet build skid-monitor-client/bindings/dotnet/examples/Skid.Monitor.Client.SampleExtension/Skid.Monitor.Client.SampleExtension.csproj
```

Start the client with the .NET host:

```sh
SKID_MONITOR_DOTNET_EXTENSIONS=./skid-monitor-client/bindings/dotnet/examples/Skid.Monitor.Client.SampleExtension/bin/Debug/net8.0/Skid.Monitor.Client.SampleExtension.dll \
SKID_MONITOR_EXTENSION_HOST="dotnet run --project skid-monitor-client/bindings/dotnet/Skid.Monitor.Client.ExtensionHost/Skid.Monitor.Client.ExtensionHost.csproj" \
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
