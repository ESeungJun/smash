# 코드 규칙표

이 저장소의 코드 규칙마다 무엇이 막는지 적는다. 장치가 없으면 `글` 이라고 그대로 적는다.
`scripts/harness/check-rules.sh` 가 적힌 장치·시험 파일이 실제로 있는지, 장치에 시험이 있는지 확인한다(CI 에서도 돈다).

규칙이 생기는 곳은 세 군데다.
- **설계서**: `docs/설계서.md` "구조 규칙" 표(`S1`, `S2` …: 의존 방향, 모듈 경계, 금지 API). 출처 칸에 `설계서 S1` 처럼 ID 를 적는다. 빠진 ID 가 있으면 check-rules 가 실패한다. 구현 하네스를 깔 때 몇 개를 바로 장치로 만든다.
- **리뷰 지적**: review-all·사람 리뷰에서 같은 지적이 두 번 나오면 여기에 올린다.
- **사용자 지적**: 사용자가 같은 실수를 두 번 짚으면 여기에 올린다.

경로는 백틱으로 감싸고 `/` 나 `.` 이 들어가게 쓴다(예: `lint.config.js`, `tests/rule1.test.ts`). 칸 안에 세로선을 쓰려면 `\|`.

장치로 만드는 순서: 코드 구조·타입으로 아예 못 하게 → 린트 규칙 → 테스트 → CI → 그래도 판단이 필요하면 글.
새 장치는 **과거 실수를 시험으로 먼저 넣어 실패하는 것을 확인한 뒤** 만든다.

| 번호 | 규칙 | 출처 | 장치 | 시험 | 메모 |
|---|---|---|---|---|---|
| 1 | 파일을 옮기고 기록하는 판단(규칙·목적지·보호·기록)은 Rust(`src-tauri/src/`) 한 곳에 두고, 화면은 결과만 그린다 | 설계서 전체 아키텍처 | 글 | | 화면 쪽 순수 계산은 `test/logic.test.js`, Rust 는 `cargo test`(`npm test` 가 둘 다 돈다). 남은 이중 구현은 #3 |
| 2 | 색은 디자인 토큰으로만 쓴다(하드코딩 금지) | design-system 4절 | `scripts/harness/check-tokens.sh` | `scripts/harness/tests/check-tokens.test.sh` | 토큰 원본·기준선은 `scripts/harness/tokens.allow`(기존 위반은 `check-tokens.sh --baseline` 으로 옮김). 기존 CSS 3개는 기준선(2026-11-25, #10). 화면 색은 `:root` 변수로 쓴다 |
| 3 | 기능 지도의 코드·확인 스크립트·시나리오가 실제로 있다 | qa-check | `scripts/harness/check-map.sh` | `scripts/harness/tests/check-map.test.sh` | 지도는 `docs/기능지도.md`, 확인 스크립트는 `scripts/verify/` |

## 실수 기록

`mistake-guard` 스킬로 적는다. 같은 꼴이면 횟수만 올린다. 횟수가 2 이상이면 표 번호(위 표의 번호)나 `보류: 이유` 를 적는다. 비면 `check-rules.sh` 가 실패한다.

| 날짜 | 실수 | 어디서 발견 | 횟수 | 표 번호 |
|---|---|---|---|---|
