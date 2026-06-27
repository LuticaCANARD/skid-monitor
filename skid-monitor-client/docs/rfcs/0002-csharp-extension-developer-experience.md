# Client RFC 0001: C# Extension Developer Experience

| 항목 | 값 |
| --- | --- |
| Status | Draft |
| Created | 2026-06-27 |
| File | `skid-monitor-client/docs/rfcs/0001-csharp-extension-developer-experience.md` |
| Scope | `skid-monitor-client`, `skid-monitor-client/bindings/dotnet` |
| Protocol | newline-delimited JSON events over extension host stdin |
| Decision Type | Client extension API, C# developer workflow, Unity bridge |

## Abstract

이 문서는 C# 개발자가 `skid-monitor-client`에 기능을 붙일 때의 기본 개발 경험과 런타임 경계를
정의한다. 현재 구현은 Rust client가 `Signal` 수신과 기본 렌더링을 맡고, C# extension은 별도
.NET host 프로세스에서 newline-delimited JSON event를 받아 처리한다.

핵심 방향은 C# extension을 Rust protocol layer에 직접 embed하지 않는 것이다. extension이
실패해도 client 수신 루프와 기본 화면은 살아 있어야 하며, Unity bridge 같은 시각화 연동도 같은
sidecar 모델 위에서 확장한다.

## Decision Summary

- Rust client는 TCP 수신, `Signal` decode, 기본 렌더링, extension host process 관리를 맡는다.
- C# extension은 `Skid.Monitor.Client.Sdk`의 `ISkidMonitorExtension`을 구현한다.
- Rust client는 extension host stdin으로 `skid.monitor.extension.v1` JSON event를 한 줄씩 보낸다.
- extension host는 `SKID_MONITOR_DOTNET_EXTENSIONS`에 지정된 assembly를 load한다.
- C# extension은 raw `JsonElement`를 받아 필요한 signal만 해석하고, 무거운 처리는 비동기로 넘긴다.
- Unity 연동은 C# extension이 Unity용 WebSocket, named pipe, stdin, localhost TCP relay를 여는
  bridge 방식으로 시작한다.

## Goals

- C# 개발자가 Rust 내부 구현을 몰라도 signal event를 받을 수 있게 한다.
- extension crash나 예외가 Rust client를 같이 죽이지 않게 한다.
- Unity/VRM companion client로 이어질 수 있는 C# 친화적인 연결 지점을 둔다.
- 향후 permission, manifest, event schema versioning을 넣을 수 있는 여지를 남긴다.

## Non-Goals

- C# extension에서 client의 TCP 수신부를 직접 대체하지 않는다.
- Rust process 안에 CLR을 embed하지 않는다.
- C# SDK에서 OTLP 전체를 강타입 모델로 모두 감싸지 않는다.
- Unity player를 Tauri WebView 안에 직접 embed하는 방식을 초기 범위로 잡지 않는다.

## Current Runtime Model

```text
skid-monitor-agent
        |
        v
skid-monitor-client (Rust)
        |
        | stdin, NDJSON
        v
Skid.Monitor.Client.ExtensionHost (.NET)
        |
        +-- C# extension assembly
        +-- optional Unity bridge
```

Rust client는 `SKID_MONITOR_EXTENSION_HOST`가 설정되어 있을 때 extension host를 child process로
실행한다. 이후 signal을 받을 때마다 다음 envelope를 stdin에 한 줄씩 쓴다.

```json
{
  "schema": "skid.monitor.extension.v1",
  "type": "signal",
  "signal": {}
}
```

extension host는 이 event를 parse하고 `ISkidMonitorExtension.OnSignalAsync`로 넘긴다. 현재 SDK의
surface는 의도적으로 작다.

```csharp
public interface ISkidMonitorExtension
{
    string Name { get; }

    ValueTask OnSignalAsync(
        SkidSignalContext context,
        CancellationToken cancellationToken);
}
```

## Developer Workflow

C# extension 개발자는 SDK를 reference하고 `ISkidMonitorExtension` 구현체를 만든다. 개발 중에는
extension host와 extension assembly를 각각 build한 뒤, Rust client를 아래처럼 실행한다.

```sh
dotnet build skid-monitor-client/bindings/dotnet/Skid.Monitor.Client.ExtensionHost/Skid.Monitor.Client.ExtensionHost.csproj
dotnet build skid-monitor-client/bindings/dotnet/examples/Skid.Monitor.Client.SampleExtension/Skid.Monitor.Client.SampleExtension.csproj

SKID_MONITOR_DOTNET_EXTENSIONS=./skid-monitor-client/bindings/dotnet/examples/Skid.Monitor.Client.SampleExtension/bin/Debug/net8.0/Skid.Monitor.Client.SampleExtension.dll \
SKID_MONITOR_EXTENSION_HOST="dotnet run --project skid-monitor-client/bindings/dotnet/Skid.Monitor.Client.ExtensionHost/Skid.Monitor.Client.ExtensionHost.csproj" \
cargo run -p skid-monitor-client
```

extension은 다음 원칙을 따른다.

- `OnSignalAsync` 안에서 긴 작업을 blocking하지 않는다.
- `context.Schema`와 `context.Type`을 먼저 확인한 뒤 signal body를 해석한다.
- 알 수 없는 signal variant나 누락된 field는 무시하거나 낮은 severity로 기록한다.
- 외부 API 호출, 파일 쓰기, Unity relay 같은 side effect는 설정으로 끌 수 있게 한다.
- extension이 만든 내부 event는 schema version을 붙인다.

## Signal Handling Guidance

현재 `SkidSignalContext.Signal`은 `JsonElement`다. C# 개발자는 필요한 field만 점진적으로 읽는
방식이 안전하다. 예외가 extension host 전체를 불안정하게 만들지 않도록, parsing 실패는 extension
내에서 처리한다.

```csharp
public ValueTask OnSignalAsync(
    SkidSignalContext context,
    CancellationToken cancellationToken)
{
    if (context.Schema != "skid.monitor.extension.v1" || context.Type != "signal")
    {
        return ValueTask.CompletedTask;
    }

    Console.Error.WriteLine($"[{Name}] received signal");
    return ValueTask.CompletedTask;
}
```

강타입 helper는 SDK에 바로 크게 추가하기보다, 반복되는 pattern이 확인된 뒤 작은 reader API로
추가한다. 예를 들어 metrics source, device id, severity 후보를 추출하는 helper는 유용하지만,
OTLP 전체 모델을 SDK public API로 고정하면 변경 비용이 커진다.

## Unity Bridge Direction

Unity 개발자에게는 raw `Signal`보다 avatar state event가 더 다루기 쉽다. 따라서 C# extension은
Rust client event를 받아 Unity용 얇은 event로 변환하는 bridge 역할을 할 수 있다.

```json
{
  "schema": "skid.monitor.avatar.v1",
  "state": "thermal_warning",
  "severity": 0.72,
  "source": "edge_device",
  "title": "Rack A temperature rising",
  "attributes": {
    "device_id": "edge-01",
    "sensor": "temperature"
  }
}
```

Unity bridge의 권장 순서는 다음과 같다.

1. C# extension에서 `Signal`을 avatar state event로 요약한다.
2. Unity companion client는 WebSocket 또는 localhost TCP로 event를 구독한다.
3. Unity project는 UniVRM, Animator Controller, ScriptableObject mapping asset으로 상태를
   expression, animation clip, material tint, camera cue에 연결한다.
4. Unity player가 종료되거나 느려져도 Rust client와 dashboard는 계속 동작한다.

초기에는 Unity를 client process 안에 넣지 않는다. 별도 process로 두면 packaging, GPU 부하,
asset loading, crash isolation을 더 단순하게 관리할 수 있다.

## Packaging

개발 모드에서는 `dotnet run --project ...ExtensionHost.csproj`를 써도 충분하다. 배포 모드에서는
extension host를 framework-dependent 또는 self-contained executable로 publish하고,
`SKID_MONITOR_EXTENSION_HOST`가 그 실행 파일을 가리키게 한다.

extension assembly 목록은 `SKID_MONITOR_DOTNET_EXTENSIONS`에 platform path separator로 나열한다.
향후 manifest가 들어가면 extension 이름, version, event permission, network permission,
Unity bridge 사용 여부를 명시한다.

## Open Questions

- extension별 timeout과 backpressure를 어디에서 적용할 것인가?
- extension 예외를 개별 extension 단위로 격리할 것인가, host 단위로 재시작할 것인가?
- `skid.monitor.extension.v1` event에 client version과 source timestamp를 넣을 것인가?
- avatar state event를 Rust client가 직접 만들 것인가, C# bridge가 만들 것인가?
- Unity bridge transport의 기본값을 WebSocket으로 둘 것인가, stdin/named pipe로 둘 것인가?

## MVP Scope

1. 현재 `ISkidMonitorExtension` 계약을 유지한다.
2. sample extension을 signal counter에서 간단한 signal classifier 예제로 확장한다.
3. C# extension에서 avatar state event를 생성하는 prototype을 만든다.
4. Unity companion client가 읽을 수 있는 localhost event relay를 추가한다.
5. extension manifest와 permission 모델은 별도 client RFC로 분리한다.
