# Skid Monitor Documentation

| 항목 | 값 |
| --- | --- |
| Status | Living index |
| Applies to | v0.1.x |
| Last reviewed | 2026-07-19 |

이 디렉터리는 제품 사용법, 현재 아키텍처, 운영 절차, 설계 결정을 서로 다른 층으로 나눈다.
처음 보는 사람은 아래 순서로 읽으면 된다.

1. [Getting Started](getting-started.md): local dashboard를 실행하고 성공을 판정한다.
2. [Feature Status](feature-status.md): 구현·검증·미구현 범위를 확인한다.
3. [Architecture](architecture.md): runtime 흐름, 저장소, 신뢰 경계와 장애 동작을 이해한다.
4. [RFC Index](rfcs/README.md): 현재 구조를 선택한 이유와 forward-looking 결정을 읽는다.

## 문서 역할

- 사용자 문서는 **지금 어떻게 사용하는가**를 설명한다.
- 운영 문서는 **배포, 장애, upgrade, rollback을 어떻게 다루는가**를 설명한다.
- RFC는 **왜 이 선택을 했는가**와 아직 합의 중인 설계를 기록한다.
- crate 내부 문서는 component 책임과 개발 시나리오를 기록한다.

RFC나 roadmap 문장만으로 기능이 구현됐다고 판단하지 않는다. 현재 동작의 정준 상태는
[Feature Status](feature-status.md)이고, 최종 근거는 연결된 code와 test다.

## 시작과 제품 이해

| 문서 | 범위 |
| --- | --- |
| [Getting Started](getting-started.md) | native Solo mode 실행, 검증, 종료, 문제 해결 |
| [Feature Status](feature-status.md) | Stable / Experimental / Prototype / Planned 상태와 근거 |
| [Architecture](architecture.md) | Solo/Cloud data flow, storage, ordering, failure semantics |

## 배포와 운영

| 문서 | 범위 |
| --- | --- |
| [Cloud and Solo Deployment](cloud-solo-deployment.md) | trusted-local Solo와 PostgreSQL/OIDC Cloud 분리 |
| [Kubernetes/Talos Deployment](deployment.md) | Linux cluster 목표 topology와 아직 없는 manifest 경계 |
| [Native Agent Deployment](agent-continuous-deployment.md) | Linux/macOS/Windows service와 update 정책 |
| [PostgreSQL Components and Migrations](postgresql-components-and-migrations.md) | schema authority, RLS, migration, backup/retention 경계 |
| [OIDC Account Providers](oidc-account-providers.md) | Keycloak/generic provider claim adapter와 전환 절차 |

## Component 문서

| Component | 문서 |
| --- | --- |
| Agent | [role RFC](../skid-monitor-agent/docs/rfcs/0001-crate-role.md), [product use cases](../skid-monitor-agent/docs/usecases/README.md), [cloud export](../skid-monitor-agent/CLOUD_EXPORT.md) |
| Client/frontend | [client docs](../client/skid-monitor-client/docs/README.md), [frontend runtime](../client/skid-monitor-fe/README.md) |
| Protocol | [protocol RFCs](../skid-protocol/docs/rfcs/README.md) |
| Edge agent | [role RFC](../skid-edge-agent/docs/rfcs/0001-crate-role.md) |
| File node | [role RFC](../skid-file-node/docs/rfcs/0001-crate-role.md) |
| Compute advisor | [role RFC](../skid-compute-advisor/docs/rfcs/0001-crate-role.md) |

## 설계 결정

- [RFC 0001: Initial Skid Monitor Integration](rfcs/0001-initial-skid-monitor-integration.md)
- [RFC 0002: Extensible Edge Media Provider Contract](rfcs/0002-extensible-media-provider.md)

RFC status 의미와 index는 [RFC README](rfcs/README.md)에 정의한다.
