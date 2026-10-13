# signed clone 검증 가이드

signed clone 모드는 2026-10-14에 **예측에 기반해 구현**되었다. 지금까지 확인한 것은 정적 검사뿐이다(복제, 서명, Gatekeeper, 단위·E2E 테스트). 이 문서는 남은 실제 실행 검증을 나중에 순서대로 따라 할 수 있게 정리한 것이다. 설계 배경은 [signed-clone-design.md](signed-clone-design.md)에 있다.

**원칙**

- **테스트 전용 프로필과 테스트 계정**으로 한다. 로그인은 직접 한다.
- 이 앱은 `~/Library/Application Support/Claude`, `~/.claude`, `Claude-3p`, Keychain을 쓰지 않는다. 다만 stray 시나리오(G 항목)에서는 **Claude가 스스로** 잠시 Default 데이터로 뜬다. 지우거나 옮기는 동작은 없지만, Default 계정 화면이 잠깐 보일 수 있다.
- 각 항목의 "실패 시"는 무엇을 고치거나 보고하면 되는지 적은 것이다. 결과는 맨 아래 표에 기록한다.

---

## 0. 준비

### 0.1 빌드와 설치

```bash
pnpm install
```

```bash
pnpm --filter ai-profiles tauri build --bundles app --config '{"bundle":{"createUpdaterArtifacts":false}}'
```

결과물은 `apps/ai-profiles/src-tauri/target/release/bundle/macos/ai-profiles.app`이다. 이것을 `/Applications`에 복사하고, 처음 열 때 우클릭 → 열기를 한다(공증되지 않은 로컬 빌드이기 때문).

- 감시 에이전트와 launcher는 **지금 실행 중인 ai-profiles 실행 파일의 경로**를 기록한다. 그래서 실제 검증은 `/Applications`에 설치한 앱으로 한다. `tauri dev`로 해도 동작은 하지만 경로가 개발 빌드를 가리킨다.
- 원본 ai-profiles 앱이 설치되어 있다면 **동시에 실행하지 않는다**. 원본은 `signedClone`을 모르므로 저장할 때 이 필드를 지운다.

### 0.2 테스트 프로필과 셸 변수

1. ai-profiles에서 새 프로필을 만든다. 예: 이름 `CloneTest`, 앱 Claude, Desktop 켬, CLI 켬, Signed clone은 **끔**(C1을 먼저 하기 위해서).
2. 아래 명령으로 id를 찾는다.

```bash
plutil -p "$HOME/Library/Application Support/ai-profiles/profiles.json" | grep -E '"(id|name|signedClone|distinctDockIcon)"'
```

```bash
export ID=<위에서 찾은 id>
```

```bash
export P="$HOME/Library/Application Support/ai-profiles/profiles/$ID"
```

### 0.3 자주 쓰는 확인 명령

프로필 프로세스(복제본, 원본, stray) 보기:

```bash
ps -ax -o pid=,etime=,command= | grep -E 'app.noindex|Claude.app/Contents/MacOS/Claude' | grep -v -E 'Helper|grep'
```

복제본, 기록 파일, 버전 보기:

```bash
ls -la "$P/app.noindex" && cat "$P/app.noindex/clone.json" && /usr/libexec/PlistBuddy -c 'Print :CFBundleVersion' "$P/app.noindex/"*.app/Contents/Info.plist
```

감시 에이전트 상태:

```bash
launchctl print "gui/$(id -u)/app.ai-profiles.clone-watch" | head -20
```

감시 에이전트 로그:

```bash
tail -f "$HOME/Library/Application Support/ai-profiles/clone-watch.log"
```

복제본 서명 검사(4단계 중 3단계를 직접 확인):

```bash
codesign --verify --deep --verbose=1 "$P/app.noindex/"*.app && spctl --assess --type execute -vv "$P/app.noindex/"*.app && codesign -dvvv "$P/app.noindex/"*.app 2>&1 | grep CDHash && codesign -dvvv /Applications/Claude.app 2>&1 | grep CDHash
```

실행하지 않는 자동 검증(실제 Claude.app 복제 → 검증 → 삭제):

```bash
cd apps/ai-profiles/src-tauri && AI_PROFILES_E2E=1 cargo test a_built_clone_keeps_the_vendor_signature -- --nocapture
```

---

## 1. 먼저 할 것: 출시 판정 항목 (★)

이 다섯 개가 통과해야 모드가 쓸모 있다. 30분 정도 걸린다.

| 순서 | ID | 한 줄 요약 |
|------|----|-----------|
| 1 | C1 | script에서 signed clone으로 바꿔도 다시 로그인할 필요가 없다 |
| 2 | B2 | Code 탭이 프로필의 `cli-config`를 쓴다 |
| 3 | B4, C4 | 권한 요청이 "Claude" 이름으로 뜨고, 원본에서 준 권한이 이어진다 |
| 4 | D2 | Dock에서 프로필 색 아이콘이 보인다 |
| 5 | F1, F3 | 다음 Claude 업데이트가 나왔을 때 수행한다 |

---

## 2. 전체 체크리스트

### A. 서명, Gatekeeper, LaunchServices

**A1 — 서명 보존** (정적 검사로 이미 통과)
- 절차: 0.3의 "복제본 서명 검사" 명령을 실행한다.
- 기대 결과: `valid on disk`, `accepted / Notarized Developer ID`, 두 CDHash가 같음. `codesign --strict`는 실패하는 것이 **정상**이다(FinderInfo).

**A2 — 첫 실행 때 Gatekeeper 대화상자 없음**
- 절차: 복제본이 막 만들어진 상태에서 프로필을 연다.
- 기대 결과: 대화상자가 없다. 기동 시간을 기록한다(첫 실행 평가로 몇 초 늘어날 수 있다).
- 실패 시: `xattr -l "$P/app.noindex/"*.app` 결과를 기록한다. quarantine이 있으면 복제 후 제거하는 단계를 추가한다.

**A3 — 기본 핸들러가 원본에 남아 있음**
- 절차: 프로필을 한 번 연 뒤 아래를 실행한다.

  ```bash
  swift -e 'import AppKit; print(NSWorkspace.shared.urlForApplication(toOpen: URL(string: "claude://x")!)?.path ?? "none"); print(NSWorkspace.shared.urlForApplication(withBundleIdentifier: "com.anthropic.claudefordesktop")?.path ?? "none")'
  ```

- 기대 결과: 두 줄 모두 `/Applications/Claude.app`.
- 복제본이 원본보다 **새 버전이 된 뒤**(F1 이후)에도 다시 확인한다.
- 실패 시: 복제본 경로가 나오면 S4 위험이 실제로 있다는 뜻이다. 복제본이 종료될 때 `lsregister -u`로 등록을 해제하는 단계를 추가하는 것을 검토한다.

**A4 — Spotlight에 나오지 않음**
- 절차: 아래 명령을 실행한다.

  ```bash
  mdfind 'kMDItemCFBundleIdentifier == "com.anthropic.claudefordesktop"'
  ```

- 기대 결과: `app.noindex` 경로가 결과에 없다.

### B. 실행과 격리

**B1 — 실행 중 판정**
- 절차: 프로필을 열고 0.3의 프로세스 명령을 실행한다.
- 기대 결과: `…/app.noindex/Claude (CloneTest).app/Contents/MacOS/Claude --user-data-dir=…/gui-data` 한 줄. 끝에 다른 인자가 없다.

**B2 — env 전달 (★)**
- 절차:
  1. 실행 전에 `stat -f %Sm ~/.claude/projects`를 기록한다.
  2. 복제본의 Code 탭에서 아무 폴더로 세션을 하나 시작한다.
  3. `ls "$P/cli-config/projects"`를 확인한다.
- 기대 결과: 프로필의 `cli-config/projects`에 항목이 생기고, `~/.claude/projects`의 mtime은 그대로다.
- 실패 시: `open --env`가 앱까지 전달되지 않는 것이다. `launch::open_instance_of`를 `NSWorkspaceOpenConfiguration`(arguments·environment)으로 바꾼다.

**B3 — `-n` 필수 확인** (선택)
- 절차: 원본 Claude를 실행한 상태에서 `open -a "$P/app.noindex/"*.app`(`-n` 없이)을 실행한다.
- 기대 결과: 원본 창이 앞으로 온다. 이것으로 `-n`이 필요한 이유가 확인된다. 새 프로세스가 생기면 그 stray는 G 항목대로 정리된다.

**B4 — 권한 요청의 주체 (★)**
- 절차: 복제본에서 마이크(음성 입력)를 쓰거나 Cowork에서 폴더를 연결한다. GUI, Spotlight의 launcher, 감시 에이전트를 통한 재실행 세 경로 모두에서 확인한다.
- 기대 결과: 권한 요청 창의 앱 이름이 **"Claude"**다. ai-profiles나 `Claude (CloneTest)` launcher가 아니다.
- 실패 시: 실행이 LaunchServices를 거치지 않는 경로가 있다는 뜻이다. 해당 경로를 기록한다.

**B5 — 동시 실행**
- 절차: Default, CloneTest(signed clone), 다른 script 프로필을 동시에 실행한다.
- 기대 결과: 각 프로필의 Open 버튼이 자기 창만 앞으로 가져온다.

### C. 인증과 권한 보존

**C1 — 모드 전환 시 재로그인 없음 (★)**
- 절차:
  1. script 모드에서 테스트 계정으로 로그인한다. 처음 로그인할 때는 다른 Claude를 모두 종료한다.
  2. 프로필을 종료한다.
  3. 프로필 편집에서 Signed clone을 켜고 저장한다.
  4. 연다.
- 기대 결과: 로그인 상태가 유지되고, Keychain "Claude Safe Storage" 프롬프트가 없다.
- 실패 시: strict 검사 실패가 실행 중 검사에 영향을 주는 것이다. 이 모드를 보류한다(설계 Q3).

**C2 — CLI와 Code 탭이 같은 Keychain 항목을 씀**
- 절차: 아래 두 명령을 차례로 실행한다(비밀값은 출력하지 않는다).

  ```bash
  H=$(printf %s "$P/cli-config" | shasum -a 256 | cut -c1-8); echo "$H"
  ```

  ```bash
  security find-generic-password -s "Claude Code-credentials-$H" >/dev/null && echo present
  ```

- 기대 결과: `claude-<slug>`로 로그인한 뒤 `present`가 나오고, 복제본의 Code 탭도 로그인된 상태다.

**C3 — 기존 기능**
- 절차: 프로필 상세 화면을 확인한다.
- 기대 결과: 사용량 미터, 세션 목록, 계정 이메일이 그대로 나온다.

**C4 — TCC 권한 이어짐 (★)**
- 절차: 원본 Claude에서 이미 허용한 권한(마이크, 파일·폴더)을 복제본에서 사용한다.
- 기대 결과: 다시 묻지 않는다.

**C5 — passkey와 하드웨어 키 로그인**
- 기대 결과: 동작한다(`keychain-access-groups`가 보존되었으므로).

**C6 — Cowork 폴더 연결**
- 기대 결과: 동작한다. wrapper 모드에서 안 되던 기능이다.

### D. 아이콘과 표시

**D1 — launcher 아이콘**
- 기대 결과: Spotlight와 Finder에서 `Claude (CloneTest)`가 배지 아이콘으로 보인다.

**D2 — 실행 중 Dock 아이콘 (★)**
- 절차: 프로필을 연다. 시스템 설정 → 외관 → 아이콘 및 위젯 스타일을 바꿔 가며 본다.
- 기대 결과: Dock 타일이 프로필 색 배지 아이콘이다. 원본 아이콘이 그대로 보이면 "실패"로 기록한다(설계 Q4: 모드는 유지).

**D3 — 이름 표시**
- 기대 결과: Dock 레이블, ⌘-Tab, 메뉴 막대에 각각 어떤 이름이 보이는지 기록한다(`Claude (CloneTest)`인지 `Claude`인지).

**D4 — 실행 중 색 변경**
- 절차: 프로필이 실행 중일 때 색을 바꾼다. 종료한 뒤 다시 연다.
- 기대 결과: 실행 중에는 복제본의 mtime이 바뀌지 않고(`stat -f %m "$P/app.noindex/"*.app`), 다시 열면 새 색이 보인다.

### E. 실행 중 활성화

**E1** — 실행 중일 때 launcher나 GUI로 다시 열면 새 프로세스 없이 그 프로필 창이 앞으로 온다(0.3 명령으로 프로세스가 1개인지 확인).

**E2** — ⌘W로 창을 모두 닫은 뒤 다시 열면 창이 다시 생긴다.

**E3** — 다른 프로필이나 Default 창은 활성화되지 않는다.

### F. 업데이트 (Claude 업데이트가 나왔을 때)

**F1 — 복제본 자가 업데이트 (★)**
- 절차: CloneTest를 실행해 둔 채로 Claude의 업데이트 알림이 오면 재시작해 업데이트한다. 로그를 띄워 둔다.
- 기대 결과: 업데이트가 끝까지 성공한다. App Management 프롬프트가 없다. 복제본 버전이 올라간다.

**F2 — 업데이트 재실행이 정리됨**
- 기대 결과: 재실행된 stray가 20초 안에 종료되고 프로필로 다시 열린다(로그에 `reopening it`). 아이콘은 복제본 버전이 원본보다 높으면 그 자리에서, 같으면 다시 복제해서 복원된다.

**F3 — Default가 실행 중일 때 업데이트 (★)**
- 절차: Default Claude를 실행해 둔 상태에서 F1을 한다.
- 기대 결과: stray가 Default로 넘겨주고 즉시 종료하는지 기록한다. 약 30초 뒤 "Claude updated itself in this profile…" 알림이 오는지 본다.

**F4 — ShipIt 캐시 공유**
- 절차: 원본과 복제본이 동시에 업데이트를 받을 때 `~/Library/Caches/com.anthropic.claudefordesktop.ShipIt/`의 상태 파일이 어떤 대상 경로를 가리키는지 기록한다.

**F5 — 원본 업데이트 후 앞으로만 갱신**
- 절차: `/Applications/Claude.app`이 업데이트된 뒤 프로필을 연다.
- 기대 결과: `clone.json`의 `clonedFromVersion`과 복제본 버전이 새 버전이 된다. 복제본이 원본보다 새 버전인 경우에는 다시 복제되지 않는다(버전이 내려가지 않음).

**F6 — root 소유 파일**
- 절차: 업데이트 후 `find "$P/app.noindex" ! -user "$USER" | head`를 실행한다.
- 기대 결과: 파일이 있으면 프로필을 열 때 "belong to another user…" 오류와 해결 명령이 안내된다.

**F7 — App Management**
- 기대 결과: 한 번 실행된 복제본에 아이콘을 다시 붙이거나, 이전 복제본을 옮기거나 지울 때 차단되거나 프롬프트가 뜨는지 기록한다. 지우지 못한 이전 복제본은 다음에 열 때 정리되는지 본다(`ls -la "$P/app.noindex"`의 `.replaced-*`).

### G. Dock과 로그인 시 재실행

**G1 — 고정한 Dock 타일**
- 절차: 실행 중인 복제본의 Dock 타일에서 우클릭 → 옵션 → Dock에 유지를 선택한다. 프로필을 종료한 뒤 그 타일을 클릭한다.
- 기대 결과: 잠깐 Default 데이터로 뜬다. 20초 안에 종료되고 CloneTest로 다시 열린다.

**G2 — 로그인 시 창 복원**
- 절차: 프로필을 열어 둔 채 "다시 로그인할 때 윈도우 다시 열기"를 켜고 로그아웃했다가 다시 로그인한다.
- 기대 결과: 에이전트가 정리한다.

**G3 — `claude://` 링크**
- 절차: 실행 중인 Claude가 없을 때 `open "claude://"`를 실행한다.
- 기대 결과: 어느 번들이 실행되는지 기록한다(S4).

**G4 — 오래된 stray는 묻는다**
- 절차:
  1. 에이전트를 잠시 끈다.

     ```bash
     launchctl bootout "gui/$(id -u)/app.ai-profiles.clone-watch"
     ```

  2. G1처럼 타일을 클릭하고 30초 이상 기다린다.
  3. 에이전트를 다시 켠다.

     ```bash
     launchctl bootstrap "gui/$(id -u)" "$HOME/Library/LaunchAgents/app.ai-profiles.clone-watch.plist"
     ```

- 기대 결과: 자동으로 종료되지 않고 "Running without its profile…" 알림만 온다. ai-profiles에서 Open을 누르면 "Quit and reopen" 토스트가 뜨고, 누르면 정리된다. Spotlight의 launcher로 열면 확인 대화상자가 뜬다.

**G5 — 백그라운드 항목**
- 절차: 에이전트를 처음 설치할 때 macOS의 "백그라운드 항목 추가" 알림을 확인한다. 시스템 설정 → 일반 → 로그인 항목 및 확장 프로그램에서 끄고 0.3의 상태 명령을 실행한다.
- 기대 결과: 꺼진 상태로 나온다.

### H. 기존 기능 회귀

**H1** — signed clone 프로필에서 세션 이동, 보관, 복원이 동작한다. "인스턴스 종료" 확인을 누르면 복제본이 종료된다.

**H2** — stray가 떠 있을 때 Default와 주고받는 세션 이동이 stray를 Default 인스턴스로 보고 종료를 제안한다.

**H3** — script와 wrapper 모드 프로필을 생성하고 열고 편집하는 동작이 전과 같다.

**H4** — 데이터 경로 불변:
- 절차: 모드를 바꾸기 전과 후에, 실행하지 않은 상태에서 아래 명령을 실행해 비교한다.

  ```bash
  find "$P/gui-data" "$P/cli-config" -maxdepth 1 -exec stat -f '%i %N' {} + | sort
  ```

  모드는 script → signed clone → script → wrapper → signed clone 순으로 바꾼다.
- 기대 결과: 파일 목록과 inode가 같다.

**H5** — wrapper ↔ signed clone 전환 시 재로그인이 필요한지 기록한다.

**H6** — 실행 중인 signed clone 프로필을 삭제할 수 있다(기존 정책). 삭제한 뒤 에이전트가 그 프로필을 다시 띄우지 않는다.

### I. 안전 경계

**I1** — 검증 중 다른 터미널에서 아래 명령을 실행해 둔다.

```bash
sudo fs_usage -w -f filesys | grep -E 'ai-profiles|launcher' | grep -E 'Application Support/Claude[/ ]|Claude-3p|/\.claude'
```

- 기대 결과: 우리 프로세스가 그 경로에 쓴 기록이 없다.

**I2** — 복제본이 실행 중일 때 색 변경, 이름 변경, 모드 전환을 시도해도 복제본 번들이 바뀌지 않는다(mtime과 inode로 확인).

---

## 3. 문제가 생겼을 때 되돌리기

- **모드 끄기**: 프로필 편집에서 Signed clone을 끄면 script launcher로 돌아간다. 실행 중이 아니면 복제본도 지워지고, 데이터는 그대로다.
- **에이전트 수동 제거**: 아래 명령을 실행한다.

  ```bash
  launchctl bootout "gui/$(id -u)/app.ai-profiles.clone-watch"; rm -f "$HOME/Library/LaunchAgents/app.ai-profiles.clone-watch.plist"
  ```

- **복제본 수동 삭제**: 프로필을 종료한 뒤 아래 명령을 실행한다. 복제본은 캐시라서 다음에 열 때 다시 만들어진다.

  ```bash
  rm -rf "$P/app.noindex"
  ```

---

## 4. 결과 기록

| ID | 결과 (통과/실패/보류) | 날짜 | macOS / Claude 버전 | 메모 |
|----|----------------------|------|--------------------|------|
| A1 | 통과 (정적) | 2026-10-14 | 26.6.2 / 2.31226.1 | `--strict`만 실패 (예상대로) |
| A2 | | | | |
| A3 | | | | |
| A4 | | | | |
| B1 | | | | |
| B2 ★ | | | | |
| B4 ★ | | | | |
| B5 | | | | |
| C1 ★ | | | | |
| C2 | | | | |
| C3 | | | | |
| C4 ★ | | | | |
| C5 | | | | |
| C6 | | | | |
| D2 ★ | | | | |
| D3 | | | | |
| D4 | | | | |
| E1–E3 | | | | |
| F1 ★ | | | | |
| F2 | | | | |
| F3 ★ | | | | |
| F4 | | | | |
| F5 | | | | |
| F6 | | | | |
| F7 | | | | |
| G1–G5 | | | | |
| H1–H6 | | | | |
| I1–I2 | | | | |
