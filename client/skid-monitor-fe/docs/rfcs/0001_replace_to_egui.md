# Tauri에서 egui로 변경함

## 결정

`client/skid-monitor-fe`는 Tauri/Svelte frontend가 아니라 egui 기반 native Rust desktop app으로 운영함.

## 근거

1. 이 앱의 역할은 Skid Monitor 신호를 실시간으로 훑는 control room이다.
2. 주요 화면은 metric/log/trace 상태, source별 카운터, 최근 이벤트 목록처럼 밀도 있는 운영 UI다.
3. 이 범위는 WebView보다 Rust immediate-mode UI가 더 단순하고 배포 의존성도 작다.
4. rich VRM 연출은 client의 Unity binding 확장 경로로 남긴다. 단, 운영 상태를 보여주는 native
   high-spec VRM/MToon, expression/SpringBone과 다중 VRMA crossfade viewport는
   [RFC 0003](0003-vrm-avatar-presenter.md)에 따라 egui frontend 안에 격리한다.
5. rich한 웹 그래프나 외부 plugin panel이 필요해지면 별도 웹 frontend를 새로 둔다.

## 결과

- `client/skid-monitor-fe`는 workspace Rust crate가 된다.
- TCP signal bind/read는 기존 `skid-monitor-client::receiver`를 재사용하고, 앱용 receive loop는
  `skid-monitor-client::receiver_loop`를 사용한다.
- Tauri/Svelte, Bun, Vite, WebKitGTK 의존성은 frontend 경계에서 제거한다.
