<p align="center"><img src="docs/images/icon.png" width="120" alt="Smash!"></p>

<h1 align="center">Smash! · 등짝 스매시</h1>

<p align="center">내가 정한 규칙대로 다운로드 폴더를 정리하는 Mac·Windows 앱.<br>
정리하기를 누르면 화면에 커다란 손바닥이 내려와 "찰싹!!" 하고, 파일은 제자리로 들어가 있다.</p>

<p align="center">
  <a href="https://github.com/ESeungJun/smash/releases/latest/download/Smash-macOS.dmg"><b>⬇ 맥 내려받기</b></a> ·
  <a href="https://github.com/ESeungJun/smash/releases/latest/download/Smash-Windows-x64-Setup.exe"><b>⬇ 윈도우 (x64) 내려받기</b></a><br>
  <sub>macOS 11 이상 (애플 칩·인텔) · Windows 10 이상 · 맥 앱은 Apple 서명·공증 · <a href="https://github.com/ESeungJun/smash/releases">모든 릴리스</a></sub>
</p>

<p align="center"><a href="README.md">English</a> · <b>한국어</b></p>
<p align="center">📝 만든 이야기: <a href="https://velog.io/@haul/smash-downloads-folder-slap">velog</a> · <a href="https://velog.io/@haul/electron-to-tauri-198mb-to-5mb">Tauri 전환기</a></p>

<p align="center"><a href="docs/video/smash.mp4"><img src="docs/images/smash.gif" width="100%" alt="Smash!: 정리하기를 누르면 화면에 커다란 손바닥이 '찰싹!!' 내려오고, 다운로드 목록이 비어 있다"></a><br>
<sub>▶ <a href="docs/video/smash.mp4">고화질 영상 보기 (MP4, 4초)</a></sub></p>

---

"방 좀 치워라!" 엄마 손바닥이 등짝에 날아오기 직전에 듣던 말이다.
다운로드 폴더도 그 정도는 맞아야 할 것 같았다. 스크린샷, 설치 파일, PDF, 영상이 쌓여서 뭐 하나 찾기가 힘들었다.

Smash!는 한 번 정해 둔 규칙대로 파일을 내가 고른 폴더에 옮긴다.
옮기기 전에 무엇이 어디로 갈지 전부 보여 주고, 영구 삭제는 하지 않는다.
API 키 없이 내 컴퓨터에 로그인된 Claude와 같이 쓴다.

## 쓰는 법

1. 정리할 폴더를 고른다. 다운로드와 문서는 처음부터 들어 있다.
2. 미리보기를 본다. 정리한 뒤의 폴더 모양을 트리로 보여 주고, 새로 생길 폴더·중복·이름이 바뀌는 파일을 표시한다.
3. 그대로 둘 파일은 체크를 풀고 **짝! 정리하기**를 누른다.
4. 손바닥이 내리치는 순간 파일이 옮겨지고, 파일이 들어간 폴더가 열린다.

<p align="center"><img src="docs/images/hero.png" width="100%" alt="Smash!: 다운로드 파일이 문서 폴더 어디로 들어갈지 트리로 보여 주는 미리보기"></p>

## 기능

- **읽히는 규칙**: 파일 종류(사진·동영상·문서·압축·설치 파일 등)를 고르거나 확장자를 넣고, 이름에 든 단어와 "N일 지난"을 더한다.
  할 일은 폴더로 옮기기 또는 휴지통으로 보내기. 만드는 동안 지금 폴더에서 맞는 파일 수를 바로 보여 준다.
- **말로 만들기**: "이름에 '계약서'가 들어간 PDF는 문서/계약서로 옮겨줘"라고 쓰면 Claude가 규칙을 채운다.
- **문서 폴더 기준 기본 규칙**: 다운로드 파일을 `문서/사진·이미지`, `미디어`, `자료`, `개발리소스`로 모은다. 7일 지난 설치 파일은 휴지통으로 보낸다.
- **자동 정리**: 켜 둔 폴더에 새 파일이 들어오면 다 받아질 때까지 기다렸다가(크기가 두 번 그대로인지 확인) 정리한다.
  창을 닫아도 메뉴 막대(윈도우는 알림 영역)에서 계속 지켜본다.
  끝나면 화면 위쪽에 "으이구!! 폴더 청소 좀 하랬지!!" 배너가 내려온다. 누르면 정리한 폴더가 열린다.

  <img src="docs/images/nag.png" width="100%" alt="화면 위쪽에 내려온 자동 정리 배너: 으이구!! 폴더 청소 좀 하랬지!! 와 정리한 파일 수">
- **등짝 스매시**: 화면 전체를 덮는 투명 창에 손바닥이 나타나 힘을 모았다가 내리치고, 화면과 앱 창이 들썩인다.
  마우스는 그대로 통과한다. 효과와 폴더 열기는 설정에서 따로 끌 수 있다.
- **안전장치**
  - 영구 삭제 없이 시스템 휴지통으로만 보낸다.
  - 같은 이름이 있으면 덮어쓰지 않고 ` (2)`를 붙인다. 내용까지 같으면 중복으로 보고 휴지통으로 보낸다.
  - 숨김 파일, 다운로드 중인 파일, 폴더(git 저장소 포함)는 건드리지 않는다.
  - 설정한 폴더 밖으로는 옮기지 않는다.
  - macOS가 한글 파일 이름을 자모로 풀어 저장해도(NFD) 규칙 단어와 맞춰 비교한다. 그래서 "스크린샷" 규칙이 실제로 스크린샷을 잡는다.
- **기록·되돌리기**: 마지막 정리를 한 번에 되돌린다.

## Claude 연동 (API 키 없음)

- **이건 어디로 갈까?**: 규칙에 안 걸린 파일은 `claude -p`(도구 끔)에 물어본다. 이름·크기·날짜와 폴더 목록만 보내고 파일 내용은 보내지 않는다.
- **Claude Code·Claude Desktop용 MCP 서버**: "자동 정리 · Claude" 탭에서 등록한다.
  `CLAUDE_CONFIG_DIR`로 쓰는 다른 설정 위치까지 찾아 모두 등록하고, Claude Desktop 설정은 먼저 `.bak`으로 백업한다.
  그다음 Claude에게 "다운로드 폴더 정리해 줘"라고 하면 앱과 같은 규칙과 기록으로 정리한다.

서버 이름은 `smash`, 도구는 `list_targets`, `scan_folder`, `apply_plan`, `add_rule`, `undo_last`, `history` 6개다.
Claude 기능은 [Claude Code CLI](https://docs.claude.com/en/docs/claude-code)에 로그인되어 있어야 쓸 수 있고, 나머지는 없어도 된다.

## 설치

### 맥

[`Smash-macOS.dmg`](https://github.com/ESeungJun/smash/releases/latest/download/Smash-macOS.dmg)를 내려받는다.
파일 하나로 애플 칩과 인텔 맥 모두에서 돈다.
dmg를 열고 `Smash.app`을 응용 프로그램 폴더로 끌어 넣는다. Apple 서명·공증을 받아서 경고 없이 열린다.

새 버전은 알아서 받아 둔다. 다 받으면 트레이 메뉴에 **새 버전 vX.Y.Z 설치하고 다시 시작**이 생긴다.

1.x에서 Claude와 연결해 뒀다면, 2.0을 처음 켤 때 그 연결을 새 앱으로 알아서 바꿔 준다.

### 윈도우

[`Smash-Windows-x64-Setup.exe`](https://github.com/ESeungJun/smash/releases/latest/download/Smash-Windows-x64-Setup.exe)를 실행한다.
코드 서명 전이라 SmartScreen 경고가 뜨면 **추가 정보 → 실행**을 누른다.
Windows 10 이상. 실제 윈도우 기기에서는 아직 테스트하지 않았다.

화면은 한국어와 영어를 지원한다. 처음에는 시스템 언어를 따르고, **자동 정리 · Claude → 언어 · Language**에서 바꿀 수 있다.

## 만든 재료

- **Tauri 2 + Rust**: 앱 본체, 규칙 엔진, 폴더 감시. 화면은 프레임워크 없는 HTML·CSS·JavaScript다.
  스매시 효과와 배너는 포커스를 가져가지 않는 투명 창이다.
- **stdio MCP**: Claude가 앱 실행 파일을 `smash --mcp`로 띄워서 따로 깔 게 없다.
- **cargo test**로 규칙 엔진, MCP 서버, 자동 정리 감시를 테스트한다.
- Electron(1.x)에서 Tauri(2.0)로 옮기면서 맥 설치 파일이 120MB 안팎에서 몇 MB로 줄었다.

Claude Code와 함께 만들었다.

## 개발

```bash
npm install
npm run dev        # 앱 실행 (Rust 필요)
npm test           # cargo test (아래처럼 --test-threads=1 로)
npm run build      # 이 컴퓨터용 설치 파일
npm run icons      # assets/*.svg → build/icon.png, 트레이 아이콘
```

규칙 엔진은 `src-tauri/src`(`organizer.rs`, `watcher.rs`, `claude.rs`, `mcp.rs`), 화면은 `src/renderer`에 있다.
`src/renderer/tauri-bridge.js`가 Rust 명령을 화면에 `window.tidy`로 넘긴다.
`feature/*` 브랜치를 push하면 GitHub Actions가 테스트를 돌리고 서명 없는 설치 파일을 만든다. `vX.Y.Z` 태그는 서명한 릴리스를 만든다.

- 설정: 맥 `~/Library/Application Support/Smash/config.json`, 윈도우 `%APPDATA%\Smash\config.json`. `SMASH_HOME`으로 위치를 바꿀 수 있다(테스트용). 테스트끼리 이 설정을 같이 쓰므로 하나씩 돌린다(`-- --test-threads=1`).
- productName은 영문 `Smash`로 둔다. Finder·메뉴 막대에 보이는 "Smash!"는 `build/lproj`가 맡는다.

### 브랜치 (git flow)

- `main`: 릴리스된 코드만. 릴리스마다 `v1.0.0` 같은 태그를 단다.
- `develop`: 다음 릴리스를 모으는 곳. 기능은 `feature/이름`에서 만들어 `develop`에 합친다.
- 릴리스: `develop`에서 `release/x.y.z`를 따서 `package.json` 버전을 올리고, `main`에 합쳐 태그를 단 뒤 `develop`에도 다시 합친다.
- 급한 수정: `main`에서 `hotfix/x.y.z`를 따서 고치고 `main`·`develop` 둘 다에 합친다.
- 개발 중에는 버전을 올리지 않고 설치본만 바꾼다. 버전은 릴리스할 때만 올린다.
