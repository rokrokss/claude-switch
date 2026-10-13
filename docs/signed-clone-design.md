# signed_clone 실행 모드 (macOS Claude Desktop 전용)

- 상태: **구현 완료, 실제 실행 검증 대기** (2026-10-14)
- 기준 코드: ai-profiles `54b8259` 위에 구현. 설계는 claude-desktop-profiles(cdp) `3ad67e7`을 참고했다.
- 측정 환경: macOS 26.6.2 (25G83), arm64, Claude Desktop 2.31226.1
- 검증 절차: [signed-clone-verification.md](signed-clone-verification.md)

---

## 0. 요약

Claude 프로필마다 `/Applications/Claude.app`의 **APFS 복제본**을 하나씩 두고, 그 복제본으로 Claude Desktop을 실행한다. 복제본 안의 파일은 한 바이트도 바꾸지 않는다. 그래서 Anthropic 서명(CDHash, 서명 요건)이 그대로 남고, Keychain·권한(TCC)·passkey·Cowork가 원본과 똑같이 동작할 것으로 예상한다. 프로필끼리는 Finder 사용자 지정 아이콘(프로필 색 배지)으로 구분한다. 이 아이콘은 번들 디렉터리에 붙는 메타데이터라 봉인된 내용에 포함되지 않는다.

기존 두 모드는 그대로 남는다.

| 모드 | 저장 필드 | 무엇이 실행되나 |
|------|-----------|-----------------|
| script launcher (기본) | 둘 다 false | `/Applications/Claude.app` |
| wrapper (재서명) | `distinctDockIcon: true` | `/Applications/Claude (<Name>).app` (ad hoc 재서명한 복제본) |
| **signed clone (신규)** | `signedClone: true` | `profiles/<id>/app.noindex/Claude (<Name>).app` (서명을 건드리지 않은 복제본) |

---

## 1. 전제와 결정

| # | 내용 | 결정 |
|---|------|------|
| P1 | claude-switch는 ai-profiles를 그대로 가져와 그 위에 모드를 하나 더한다. | `54b8259`를 그대로 import한 커밋을 두고, 그 위에 변경 커밋을 쌓았다. MIT 고지를 유지한다. |
| P2 | 데이터 경로와 식별자는 ai-profiles 것을 쓴다(`~/Library/Application Support/ai-profiles/`, `app.ai-profiles.*`). | 유지한다. 기존 프로필을 옮기지 않아도 된다. |
| P3 | signed clone은 Claude 프로필, macOS에서만 쓸 수 있다. | Rust `AppSpec.supports_signed_clone`과 TS `appSpecs[app].signedClone`으로 제한한다. |
| P4 | 기존 script와 wrapper 모드의 코드와 동작은 바꾸지 않는다. | 유지한다. |
| Q2 | 감시 에이전트를 기본으로 설치할지 | signed clone 프로필이 하나라도 있으면 설치한다. 자동 처리는 생긴 지 20초 이내인 stray만 한다. |
| Q3 | `codesign --strict` 실패를 받아들일지 | 받아들인다. 실행 중 검사(C1, C4, F1)에서 문제가 나오면 모드를 보류한다. |
| Q4 | macOS 26에서 Dock 아이콘(D2)이 반영되지 않아도 출시할지 | 출시한다. 서명·권한 보존이라는 이점은 남는다. |
| U1 | Claude Desktop이 업데이트되어도 이 프로그램은 업데이트할 필요가 없어야 한다. | §5.8 |

다음 대상은 **절대 건드리지 않는다**: `~/Library/Application Support/Claude`, `~/.claude`, `~/.claude.json`, `~/Library/Application Support/Claude-3p`(이 Mac에 존재), Keychain 항목, 각 프로필의 `gui-data`와 `cli-config` 내용, 그리고 `/Applications/Claude.app`(읽기만 한다).

---

## 2. 바뀌지 않는 것

| 기능 | 사용하는 경로 | signed clone에서 |
|------|---------------|------------------|
| 프로필 저장 | `profiles.json`, `profiles/<id>/gui-data`, `profiles/<id>/cli-config` | 같다. `signedClone` 필드 하나만 추가된다. |
| Desktop 로그인 | `--user-data-dir=<gui-data>`와 Keychain "Claude Safe Storage" | 같은 `gui-data`를 쓴다. 서명 요건이 같으므로 Keychain ACL을 통과할 것으로 예상한다(검증 C1). |
| CLI·Code 탭 로그인 | `CLAUDE_CONFIG_DIR=<cli-config>`로 정해지는 Keychain `Claude Code-credentials-<sha256(cli-config)[:8]>` | CLI wrapper와 **같은 함수** `paths::cli_config_dir(id)`로 경로 문자열을 만든다. 따라서 Keychain 항목도 같다(검증 C2). |
| 사용량 조회 | `usage/credentials.rs`가 `cli-config`를 읽는다. | 변경 없음 |
| 세션 목록·이동·보관 | `sessions/home.rs`가 `profiles::paths()`를 쓴다. | 경로는 변경 없음. 실행 중 판정에 stray를 포함하도록 보강했다(§5.6). |
| 계정 표시 | `accounts.rs` | 변경 없음 |

---

## 3. 측정한 사실

### 3.1 정적 프로브 (실행하지 않음)

`/Applications/Claude.app`을 `*.noindex` 폴더로 `cp -Rc` 복제하고 `NSWorkspace.setIcon`을 적용한 뒤 확인했다.

| 항목 | 결과 |
|------|------|
| `cp -Rc` | 1.35초. `/Applications`와 `~/Library`가 같은 볼륨(disk3s5)에 있다. |
| 복제되는 xattr | `com.apple.macl`, `com.apple.provenance` |
| 아이콘 적용 후 번들 루트 | `Icon\r` 파일이 생기고, 번들 디렉터리에 `com.apple.FinderInfo`가 붙는다. |
| CDHash, 서명 요건(DR) | 원본과 **같다** |
| `codesign --verify`, `--verify --deep`, `-R=<원본 DR>` | **통과** |
| `spctl --assess --type execute` | **accepted** (Notarized Developer ID, Anthropic PBC Q6L2SF6YDW) |
| `codesign --verify --strict` | **실패**: `Disallowed xattr com.apple.FinderInfo` |

### 3.2 구현 후 E2E 테스트 (실행하지 않음)

`launchers::clone::tests::a_built_clone_keeps_the_vendor_signature_and_carries_its_icon`를 `AI_PROFILES_E2E=1`로 실행해 통과를 확인했다(8.3초). 실제 `clone::ensure`가 복제본을 만들고, 아이콘을 붙이고, 4단계 검증을 통과하는 것을 확인한다. 이어서 두 번째 `ensure` 호출에서는 아무것도 바뀌지 않고(inode가 같음), 마지막에 복제본을 제거하는 것까지 본다.

정리하면, 서명 자체는 보존된다. 다만 strict 검사 기준에서는 "번들 루트에 메타데이터가 덧붙은" 상태다. 실행 중 검사(Keychain, TCC, Squirrel 업데이트)가 이것을 문제 삼는지는 실제 실행으로 확인해야 한다.

---

## 4. 모드 비교

| | script | wrapper | **signed clone** |
|---|---|---|---|
| 코드 서명 | Anthropic | ad hoc, 팀 entitlements 제거 | Anthropic. 바이트 동일하고 `--strict`만 실패 |
| Bundle ID | Anthropic | `app.ai-profiles.claude.profile.<id>` | Anthropic (원본과 공유) |
| 실행 중 Dock 아이콘 | 기본 주황 | 프로필 배지 | Finder 사용자 지정 아이콘. macOS 26 반영은 검증 D2 |
| 모드 전환 시 재로그인 | — | 필요 | 필요 없을 것으로 예상 (C1) |
| TCC·passkey·Cowork | 정상 | 권한을 다시 묻고, passkey가 안 될 수 있고, Cowork 폴더를 못 쓴다 | 정상일 것으로 예상 (C4~C6) |
| 자가 업데이트 | 원본이 업데이트된다 | 불가. 원본 버전이 바뀌면 다시 빌드한다 | 복제본 안에 설치된다. 원본보다 앞설 수 있다 |
| Dock·업데이트로 재실행될 때 | 원본이 인자 없이 뜬다 (기존 한계) | shim이 인자를 다시 넣는다 | 복제본이 인자 없이 뜬다(stray). §5.6에서 처리한다 |

---

## 5. 구조

### 5.1 데이터 모델

- `Profile.signed_clone: bool`. JSON 이름은 `signedClone`이고 기본값은 false다. `ProfilePatch.signed_clone: Option<bool>`도 추가했다.
- `profiles::validate_desktop_mode`가 생성(`create`)과 수정(`patched`) 때 다음을 검사한다.
  - 앱 spec이 `supports_signed_clone`이어야 하고, macOS여야 한다.
  - `distinct_dock_icon`과 동시에 켤 수 없다.
- `profiles.json`을 손으로 고쳐 두 값이 모두 true가 되면, 실행·launcher 생성 모두 signed clone을 우선한다.
- **하위 호환**:
  - 원본 ai-profiles나 이전 빌드는 `signedClone`을 무시한다. 그래서 그 프로필을 script launcher로 다시 만들어 실행하고, 데이터 경로는 같다.
  - 다만 그런 빌드가 `profiles.json`을 저장하면 이 필드가 사라진다. 따라서 두 앱을 같은 데이터에 함께 쓰지 않는다.
- 프런트엔드에서는 `signedClone?`을 선택 필드로 두었다. 폼은 켜져 있을 때나 끄는 중일 때만 이 값을 보낸다. 그래서 다른 프로필의 폼은 기존과 똑같은 값을 제출한다.

### 5.2 파일 배치

```
~/Library/Application Support/ai-profiles/
  profiles.json                          # signedClone 필드 추가
  clone-watch.log                        # 감시 에이전트 로그
  profiles/<id>/gui-data/                # 그대로
  profiles/<id>/cli-config/              # 그대로
  profiles/<id>/app.noindex/             # 신규 (캐시. 지워도 다음 실행 때 다시 만든다)
      Claude (<Name>).app                # 복제본 + Finder 아이콘
      clone.json                         # {clonedFromVersion, color, builtBy}
      .lock                              # 프로세스 간 잠금
/Applications/Claude (<Name>).app        # script launcher 형태. 본문은 ai-profiles로 넘기는 handoff
~/Library/LaunchAgents/app.ai-profiles.clone-watch.plist   # signed clone 프로필이 있을 때만
```

- **소유 판정은 경로로 한다.** 복제본은 Anthropic 식별자를 그대로 가지고 Info.plist가 봉인되어 있다. 그래서 기존처럼 식별자(`is_ours`)로 판정할 수 없고, 부가 정보도 번들 안에 쓸 수 없다. 대신 번들 밖의 `clone.json`에 기록한다.
- **`.noindex`**: Spotlight가 이 폴더를 색인하지 않게 해서, 인자 없이 복제본을 여는 경로를 줄인다.
- **파일 이름 `Claude (<Name>).app`**: Dock과 ⌘-Tab 레이블로 쓰일 것으로 예상한다(검증 D3). 프로필 이름을 바꾸면 다음 `ensure`에서 복제본 폴더 이름을 바꾼다(rename). 새로 복제하지 않으므로 업데이트된 버전을 잃지 않는다.

### 5.3 실행 흐름

```
GUI "Open"   ──┐
Spotlight의 launcher ──> ai-profiles --open-profile <id> ──┐
감시 에이전트 ─────────────────────────────────────────────┴─> launch::open_profile_with
  1. stray 정리 (clone::settle_strays)
     - 생긴 지 20초 이하: SIGTERM으로 종료하고 계속
     - 그보다 오래됨: AppError::StrayRunning을 반환
       · GUI는 "Quit and reopen" 토스트를 띄운다
       · launcher는 osascript 확인 대화상자를 띄운다
  2. running_pid(gui-data, 실행 파일)이 있으면 focus_pid(reopen AE + activate)하고 끝
  3. clone::ensure로 복제본을 만들거나 갱신한다 (정책은 §5.4)
  4. open -n -a <clone> --args --user-data-dir=<gui-data>, env CLAUDE_CONFIG_DIR=<cli-config>
  5. watch_start로 기동을 지켜본다 (wrapper와 같은 로직)
  6. 3~5단계가 실패하면 Bypass를 반환하고 stock 방식으로 연다 (프로필을 못 여는 일이 없게)
```

- 실행은 반드시 **LaunchServices(`open`)를 거친다.** 직접 `exec`하면 TCC의 책임 프로세스가 ai-profiles가 되어, 권한 요청이 ai-profiles 이름으로 뜬다.
- **`-n`은 필수다.** 복제본과 원본이 Bundle ID를 공유하므로, `-n` 없이 열면 실행 중인 원본이 활성화된다.
- launcher(`/Applications/Claude (<Name>).app`)가 기록된 경로에서 ai-profiles를 찾지 못하면, 기존 script 방식으로 원본을 연다.
- handoff 형태의 launcher가 signed clone이 아닌 프로필에 남아 있으면 무한 반복이 생길 수 있다. 그래서 `open_script_launcher`는 launcher를 열지 않고 stock 방식으로 연다. launcher는 다음 시작 때 올바른 형태로 다시 만들어진다(`outdated`가 형태 불일치와 host 경로 소실을 감지).

### 5.4 복제본 갱신 정책 (`clone::plan`)

| 상태 | 동작 |
|------|------|
| 복제본 없음 | 만든다 (Build) |
| 사용 중 (메인, 헬퍼, ShipIt 포함) | **건드리지 않는다** |
| 복제본 < 원본 | 다시 만든다 (버전을 앞으로) |
| 같은 버전이고 아이콘이 없거나 색이 다름 | 다시 만든다. 한 번도 실행되지 않은 새 번들에 아이콘을 붙이고, APFS 블록 공유도 되찾는다 |
| 복제본 > 원본 (자가 업데이트) | 그대로 둔다. 아이콘이 없거나 색이 다를 때만 그 자리에서 다시 붙여 본다. 실패하면 색 없이 실행한다 |
| 버전을 비교할 수 없음 | 있는 것을 그대로 실행한다 |

**만드는 절차:**

1. 프로세스 간 파일 잠금(`.lock`, `File::lock`)을 잡는다.
2. 숨김 staging 폴더(`.app` 접미사 없음)에 `cp -Rc`로 복제한다. 원본과 같은 볼륨인지 확인하고, Info.plist와 실행 파일 바이트가 같은지 비교한다.
3. 아이콘을 만든다. `icons::render_icns`(launcher와 같은 배지)를 `NSWorkspace.setIcon`으로 붙인다.
4. 검증한다. `codesign --verify --deep`(`--strict`는 쓰지 않음), 원본 DR 충족, CDHash 일치, `spctl` 통과를 모두 확인한다.
5. 바꿔치기 **직전에 사용 중인지 다시 확인**한다.
6. 기존 복제본을 `.replaced-*`로 옮기고 staging을 제자리로 rename한 뒤, 옮긴 것을 지운다. 지우기가 막히면 남겨 두고 다음 `ensure`에서 정리한다.
7. `clone.json`에 기록한다.

번들 안에 사용자 소유가 아닌 파일이 있으면(업데이트가 root로 돌았을 때) 교체를 거부한다. 해결용 `sudo` 명령은 안내만 하고 실행하지 않는다.

### 5.5 아이콘

- launcher와 같은 `render_icns`(Claude 아트워크 + 프로필 색 배지)를 쓴다.
- 실행 중에 색을 바꾸면 launcher 아이콘은 바로 바뀌고, 복제본 아이콘은 다음 실행 때 바뀐다.
- macOS 26의 Dock이 asset catalog(`CFBundleIconName`)보다 Finder 사용자 지정 아이콘을 우선하는지는 아직 모른다(검증 D2).

### 5.6 인자 유실 처리 (stray)

stray는 실행 파일이 `profiles/<id>/app.noindex/*.app/Contents/MacOS/<exec>`인데 명령줄에 `--user-data-dir=`가 없는 프로세스다(`clone::stray_profile`). 생기는 경로는 다음과 같다.

- S1: 자가 업데이트 후 ShipIt가 다시 실행함
- S2: Dock 타일이나 최근 항목 클릭
- S3: 로그인 시 창 복원
- S4: `claude://` 링크를 LaunchServices가 복제본으로 보냄
- S5: 복제본 안에서 "로그인 시 열기"를 켬
- S6: S1인데 Default 인스턴스가 이미 실행 중이어서, stray가 그쪽으로 넘겨주고 즉시 종료함

| 처리 | 위치 | 내용 |
|------|------|------|
| 실행 시점 | `launch::open_signed_clone` | 생긴 지 20초 이하인 stray는 종료하고, 그보다 오래된 것은 사용자에게 묻는다(위 §5.3). SIGKILL은 쓰지 않는다. |
| Default 실행 시점 | `commands::open_default_gui` | 먼저 fresh stray를 재배치한다. 그러지 않으면 stock 실행 요청이 stray로 넘어간다. |
| 세션 판정 | `sessions::instance::desktop_pids` | stray는 stock 데이터를 쓰므로 **Default 인스턴스로 센다.** |
| 감시 에이전트 | `clone_watch` (`ai-profiles --watch-clones`, 2초 간격) | fresh stray는 재배치한다. 오래된 stray는 pid마다 한 번 알림만 보낸다. S6은 복제본 버전이 바뀌고 30초 뒤에도 아무것도 실행 중이지 않으면 알림을 보낸다. |

- **S6을 자동으로 다시 열지 않는 이유**: Squirrel은 사용자가 그냥 종료할 때도 업데이트를 조용히 설치한다. 그래서 밖에서는 "재실행이 의도되었는지"를 구분할 수 없다. 대신 "업데이트됐다. 다시 열리지 않았으면 직접 열어라"라는 알림을 보낸다. 이 알림은 두 경우 모두에 맞는 말이다. 우리가 직접 다시 만든 복제본은 `clone.json`의 `clonedFromVersion`과 버전이 같으므로 알림을 보내지 않는다.
- **에이전트 설치와 제거**: 프로필을 생성, 수정, 삭제하거나 surface를 바꿀 때, 그리고 앱을 시작할 때 `clone_watch::sync`를 호출한다.
  - signed clone인 GUI 프로필이 하나라도 있으면 `launchctl bootstrap`으로 설치한다.
  - 하나도 없으면 `bootout`하고 plist를 지운다.
  - 실행 파일 경로가 바뀌면 plist를 다시 쓴다.
  - 사용자는 시스템 설정 → 일반 → 로그인 항목 및 확장 프로그램에서 끌 수 있다.

### 5.7 모드 전환과 되돌리기

| 전환 | 일어나는 일 | 데이터 |
|------|-------------|--------|
| script → signed clone | 복제본을 **먼저** 만든다. 실패하면 launcher를 바꾸지 않는다. 그다음 launcher를 handoff 형태로 만들고 에이전트를 설치한다. | 건드리지 않음 |
| signed clone → script 또는 wrapper | launcher를 다시 만들고, 사용 중이 아니면 복제본(캐시)을 지운다. 사용 중이면 남겨 두었다가 나중에 정리한다. 마지막 프로필이면 에이전트를 제거한다. | 건드리지 않음 |
| wrapper ↔ signed clone | wrapper가 실행 중이면 기존 `ensure_wrapper_not_running`이 거부한다. | 건드리지 않음 |
| 이름·색 변경 | launcher만 다시 만든다. 복제본 변경은 다음 실행으로 미룬다. | 건드리지 않음 |
| GUI surface 끄기 | launcher를 지우고, 사용 중이 아니면 복제본도 지운다. | 건드리지 않음 |
| 프로필 삭제 | 기존 정책을 따른다(실행 중이어도 거부하지 않음). 복제본은 프로필 폴더와 함께 휴지통으로 가거나 삭제된다. | 사용자가 선택 |

**하지 않는 일**: 계정 데이터를 자동으로 옮기거나 지우지 않는다. 실행 중인 복제본은 교체, 수정, 이동하지 않는다. `sudo`를 쓰지 않는다. `/Applications/Claude.app`에는 쓰지 않는다.

### 5.8 Claude Desktop 업데이트와 무관하게 동작

Claude Desktop이 업데이트되어도 claude-switch는 다시 빌드하거나 업데이트할 필요가 없다. Claude에 관한 값은 코드에 고정하지 않고, 필요할 때마다 설치된 앱에서 읽는다.

| 값 | 읽는 곳 |
|----|---------|
| 버전 | `CFBundleVersion` (`wrapper::bundle_version`). 실행할 때마다 비교한다 |
| 실행 파일 이름 | `CFBundleExecutable` (`clone::vendor_executable`). 복제, stray 판별, 실행 감지에 쓴다 |
| 서명 요건 | `codesign -d -r-`로 원본의 DR을 읽는다. 팀 ID를 고정하지 않는다 |
| CDHash | 원본과 복제본을 그때그때 비교한다 |
| 아이콘 | 복제할 때마다 원본 아이콘으로 다시 그린다 |

- **원본이 업데이트되면**: 실행 중이지 않은 프로필은 다음 실행 때 앞으로 다시 복제된다. 실행 중인 프로필은 그대로 두며, 복제본이 스스로 업데이트된다. 이것은 버전 비교에 따른 자동 동작이다.
- **실패해도 열린다**: 새 Claude에서 복제본을 쓸 수 없게 되면(서명 변경, 실행 위치 제한 등), 실행은 `Bypass`로 stock 방식으로 넘어가므로 프로필은 계속 열린다.
- **남아 있는 가정**: ai-profiles 전체가 기대는 것들이다. 이것들이 깨지면 원본 ai-profiles도 수정이 필요하다.
  - 앱 위치가 `/Applications/Claude.app`이다(`AppSpec.gui_bundle_candidates`).
  - Electron의 `--user-data-dir`이 동작한다.
  - Code 탭이 `CLAUDE_CONFIG_DIR`을 따른다.
- 이 앱 자신의 updater는 이 저장소의 Releases를 보도록 바꿨다. 이 저장소에는 릴리스가 없으므로 자동 업데이트도 일어나지 않는다. upstream 빌드로 덮어써지는 것을 막기 위해서다.

### 5.9 알려진 한계

- Bundle ID를 공유하므로 `claude://` 로그인 콜백은 LaunchServices가 고른 인스턴스로 간다. 프로필에 **처음 로그인할 때는 다른 Claude를 모두 종료**해야 한다(기존 모드도 마찬가지).
- 알림 설정, 기본 앱 선택, 로그인 항목이 원본과 공유된다.
- 원본과 모든 복제본이 ShipIt 캐시(`~/Library/Caches/com.anthropic.claudefordesktop.ShipIt`)를 공유한다(검증 F4).
- 자가 업데이트된 복제본은 약 650MB의 독립 사본이 된다. Time Machine 제외 설정은 하지 않았다.
- 개발 빌드(`tauri dev`)로 signed clone을 쓰면 에이전트가 개발 빌드 바이너리를 가리킨다. 설치된 앱을 실행하면 다시 바로잡힌다.

---

## 6. 코드 지도

| 파일 | 역할 |
|------|------|
| `src-tauri/src/launchers/clone.rs` | 복제본 계획(`plan`), 생성·검증(`ensure`/`build`/`verify`), 아이콘, 소유권, stray 판별과 종료 |
| `src-tauri/src/launchers/clone_watch.rs` | LaunchAgent plist, `sync`, `--watch-clones` 루프, 알림 |
| `src-tauri/src/launch.rs` | `open_profile_with`, `open_signed_clone`, `CloneEffects`/`launch_clone_with`(fallback), `open_instance_of` |
| `src-tauri/src/launchers/script.rs` | `handoff_launcher_script`, `hands_off`, `handoff_host` |
| `src-tauri/src/launchers/gui.rs` | `generate` 분기, `outdated`의 handoff 형태·host 경로 검사, `hands_off` |
| `src-tauri/src/profiles.rs` | `signed_clone` 필드, `validate_desktop_mode`, GUI surface를 끌 때 복제본 정리 |
| `src-tauri/src/paths.rs` | `profiles_root`, `clone_dir`, `clone_bundle_path`, `CLONE_DIR_NAME` |
| `src-tauri/src/sessions/instance.rs` | stray를 stock 인스턴스로 판정 |
| `src-tauri/src/commands.rs`, `cli.rs`, `main.rs`, `lib.rs`, `error.rs` | 명령 인자(`signedClone`, `quitStray`), `--watch-clones`, 시작 시 sync, `StrayRunning` |
| `src/features/profiles/components/*` | 옵션 행, `SignedCloneDialog`, `useSignedCloneOption`, 다이얼로그 연결 |
| `src/features/profiles/api/use-profile-last-used.ts` | `StrayRunning`일 때 "Quit and reopen" 토스트 |

---

## 7. 테스트

- **Rust**: 727개 통과, 그중 신규 30개. 다루는 항목은 다음과 같다.
  - 갱신 계획 표, 버전 비교, stray와 헬퍼 구분, `ps` elapsed 시간 파싱, DR·CDHash 파싱
  - 기록 파일 읽기·쓰기, 남은 임시 파일 정리, 이름 바뀐 복제본 이어받기, 사용 중인 복제본 보호
  - handoff 스크립트 생성·인식·host 경로 파싱(따옴표가 든 경로 포함), `outdated` 형태 판정
  - fallback 4가지(`FakeClone`), stray를 stock 인스턴스로 세기, 에이전트 plist·필요 여부·버전 변화 판정, `--watch-clones` 인자 파싱
- **E2E** (`AI_PROFILES_E2E=1`): `a_built_clone_keeps_the_vendor_signature_and_carries_its_icon`가 실제 복제, 검증, 정리를 확인한다. 앱은 실행하지 않는다.
- **프런트**: 689개 통과, 그중 신규 9개. 편집 계획, 생성·편집 다이얼로그 상호 배타, ChatGPT에서 미노출, 설명 대화상자를 다룬다.
- **정적 검사**: `cargo clippy -- -D warnings`(CI와 같은 형식), `cargo fmt --check`, `tsc`, `biome check`, `fallow` 모두 통과.
  - `cargo clippy --all-targets`에서 나오는 오류 2건(`test_support.rs`, `profiles.rs:788`)은 upstream 원본에도 있는 것이다.

실제로 실행하는 검증(로그인, 권한, Dock, 업데이트)은 [signed-clone-verification.md](signed-clone-verification.md)의 절차로 한다.

---

## 8. 초안 대비 바뀐 점

- `DesktopMode` enum은 만들지 않았다. bool 하나와 `validate_desktop_mode`로 충분했다.
- 두 플래그가 모두 true인 JSON을 저장할 때 자동으로 바로잡는 기능은 구현하지 않았다. 대신 실행과 생성에서 signed clone을 우선한다.
- S6 자동 재실행은 알림으로 바꿨다(이유는 §5.6). 그래서 의도하지 않은 재실행 위험이 없다.
- 프로필 삭제는 "실행 중이면 거부"가 아니라 upstream의 기존 정책(거부하지 않음)을 따른다.
- 같은 프로필에 대한 동시 `ensure`(GUI, launcher, 에이전트) 경쟁을 막기 위해 파일 잠금과 교체 직전 재확인을 추가했다.
- 앱 updater를 이 저장소로 돌리고, upstream 릴리스 워크플로를 제거했다.
