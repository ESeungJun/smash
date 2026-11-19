//! 기존 정리 패턴 시험용 가짜 폴더(#8).
//! 사람들이 문서 폴더를 실제로 나눠 쓰는 모양(종류 폴더 + 주제 폴더 + 개발 프로젝트)을 본떠 만든다.
//! 이름은 모두 일반 예시다. `demo_plan` 은 지금 엔진이 새 파일을 어디로 보내는지 표로 뽑는다(전후 비교용).

use std::fs;
use std::path::PathBuf;

use serde_json::json;

use crate::config::{default_config, save_config_value, Config};
use crate::test_util::{sandbox, write};

/// 문서 폴더에 이미 정리돼 있는 파일들 (폴더/파일)
const EXISTING: &[&str] = &[
    "자료/강의노트.pdf",
    "자료/메모.txt",
    "사진·이미지/IMG_0001.jpg",
    "사진·이미지/여행사진.jpg",
    "사진·이미지/배경.png",
    "미디어/노래.mp3",
    "미디어/영상.mp4",
    "개발리소스/아이콘·에셋/ic_home.svg",
    "개발리소스/아이콘·에셋/ic_star.svg",
    "개발리소스/아이콘·에셋/ic_user.svg",
    "개발리소스/아이콘·에셋/ic_heart.svg",
    "개발리소스/아이콘·에셋/ic_close.svg",
    "개발리소스/아이콘·에셋/loading.json",
    "개발리소스/아이콘·에셋/confetti.json",
    "개발리소스/아이콘·에셋/splash.json",
    "개발리소스/폰트/Pretendard-Bold.otf",
    "개발리소스/폰트/Pretendard-Regular.otf",
    "개발리소스/폰트/NotoSans.ttf",
    "개발리소스/설정·로그/crash-2026.log",
    "개발리소스/설정·로그/build.log",
    "세무/2025_원천징수영수증.pdf",
    "세무/2025_연말정산_공제신고서.pdf",
    "세무/2024_연말정산_공제신고서.pdf",
    "세무/홍길동_주민등록등본.pdf",
    "주거/청약 공고/행복주택_입주자모집공고.pdf",
    "주거/청약 공고/장기전세_입주자모집공고.pdf",
    "주거/청약 공고/매입임대_입주자모집공고.pdf",
    "웨딩/웨딩홀_견적.xlsx",
    "웨딩/스튜디오_계약서.pdf",
    "회사_에이컴퍼니/VOC Report - raw_1월.csv",
    "회사_에이컴퍼니/VOC Report - raw_2월.csv",
    "회사_에이컴퍼니/급여명세서_2026-05.html",
    "회사_에이컴퍼니/급여명세서_2026-06.html",
    "회사_에이컴퍼니/2026 연봉 계약서.pdf",
    "이력서/01_이력서/홍길동_이력서.pdf",
    "이력서/04_스크린샷/앱/스크린샷 2026-01-01 오후 1.00.00.png",
    "이력서/04_스크린샷/앱/스크린샷 2026-01-01 오후 1.00.10.png",
    "이력서/04_스크린샷/앱/스크린샷 2026-01-01 오후 1.00.20.png",
    "my-app/package.json",
    "my-app/src/index.js",
];

/// 다운로드·문서 폴더에 새로 들어온 파일들
pub const INCOMING: &[&str] = &[
    "다운로드/2026_원천징수영수증.pdf",
    "다운로드/임대주택_입주자모집공고.pdf",
    "다운로드/웨딩드레스_견적.pdf",
    "다운로드/VOC Report - raw_3월.csv",
    "다운로드/급여명세서_2026-07.html",
    "다운로드/ic_search.svg",
    "다운로드/lottie_heart.json",
    "다운로드/Inter-Bold.otf",
    "다운로드/스크린샷 2026-11-19 오후 3.00.00.png",
    "다운로드/IMG_2001.jpg",
    "다운로드/계약서.pdf",
    "다운로드/아무거나_보고서.pdf",
    "다운로드/package.json",
    "다운로드/tsconfig.json",
    "다운로드/docker-compose.yml",
    "다운로드/index.ts",
    "다운로드/main.py",
    "문서/회의록_0301.md",
    "문서/README.md",
];

/// 가짜 폴더를 새로 만들고, 기본 설정(다운로드 → 문서 기준 폴더 연결, 문서 종류 규칙·소분류 자동)을 그 폴더에 맞춘다.
pub fn build() -> (Config, PathBuf, PathBuf) {
    let sb = sandbox().join("pattern");
    let _ = fs::remove_dir_all(&sb);
    let _ = fs::remove_dir_all(sandbox().join("config"));
    let docs = sb.join("문서");
    let downloads = sb.join("다운로드");
    for f in EXISTING {
        write(&docs.join(f), f, 30);
    }
    for f in INCOMING {
        write(&sb.join(f), f, 0);
    }
    let mut cfg = serde_json::to_value(default_config()).unwrap();
    cfg["targets"][0]["path"] = json!(downloads);
    cfg["targets"][1]["path"] = json!(docs);
    let cfg = save_config_value(&cfg).unwrap();
    (cfg, downloads, docs)
}

/// 새 파일마다 (파일, 갈 곳 또는 그대로 둔 이유). 갈 곳은 가짜 폴더 기준 상대 경로.
pub fn demo_plan(cfg: &Config) -> Vec<(String, String)> {
    let base = sandbox().join("pattern");
    let rel = |p: &str| p.strip_prefix(&*base.to_string_lossy()).unwrap_or(p).trim_start_matches('/').to_string();
    let mut out = Vec::new();
    for t in ["downloads", "documents"] {
        let scan = crate::organizer::scan_target(cfg, Some(t)).unwrap();
        for it in &scan.items {
            let dest = if it.action == "trash" { "휴지통".into() } else { rel(&it.dest) };
            out.push((rel(&it.src), format!("{dest}  ({})", it.reason)));
        }
        for it in &scan.blocked {
            out.push((rel(&it.src), format!("[{}] {}", it.note.clone().unwrap_or_default(), it.reason)));
        }
        for f in &scan.unmatched {
            out.push((rel(&f.path), "(규칙 밖: AI 제안 대상)".into()));
        }
    }
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lang::set_lang;
    use crate::test_util::LOCK;

    /// 이미 만들어 둔 폴더를 따라가고, 코드·설정 파일은 보호하고, 새 폴더를 지어내지 않는다
    #[test]
    fn follows_existing_pattern() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_lang("ko");
        let (cfg, _, _) = build();
        let rows = demo_plan(&cfg);
        let dest = |src: &str| rows.iter().find(|(a, _)| a == src).map(|(_, b)| b.split("  (").next().unwrap().to_string()).unwrap_or_else(|| "(없음)".into());
        assert_eq!(dest("다운로드/2026_원천징수영수증.pdf"), "문서/세무/2026_원천징수영수증.pdf");
        assert_eq!(dest("다운로드/VOC Report - raw_3월.csv"), "문서/회사_에이컴퍼니/VOC Report - raw_3월.csv");
        assert_eq!(dest("다운로드/급여명세서_2026-07.html"), "문서/회사_에이컴퍼니/급여명세서_2026-07.html");
        assert_eq!(dest("다운로드/웨딩드레스_견적.pdf"), "문서/웨딩/웨딩드레스_견적.pdf");
        assert_eq!(dest("다운로드/임대주택_입주자모집공고.pdf"), "문서/주거/청약 공고/임대주택_입주자모집공고.pdf");
        assert_eq!(dest("다운로드/ic_search.svg"), "문서/개발리소스/아이콘·에셋/ic_search.svg");
        assert_eq!(dest("다운로드/Inter-Bold.otf"), "문서/개발리소스/폰트/Inter-Bold.otf");
        assert_eq!(dest("다운로드/lottie_heart.json"), "문서/개발리소스/아이콘·에셋/lottie_heart.json");
        // 근거가 없으면 규칙 목적지 그대로, 새 소분류 폴더는 만들지 않는다
        assert_eq!(dest("다운로드/계약서.pdf"), "문서/자료/계약서.pdf");
        assert_eq!(dest("다운로드/IMG_2001.jpg"), "문서/사진·이미지/IMG_2001.jpg");
        assert_eq!(dest("다운로드/스크린샷 2026-11-19 오후 3.00.00.png"), "문서/사진·이미지/스크린샷 2026-11-19 오후 3.00.00.png", "다른 주제 폴더 깊숙한 스크린샷 폴더로 가지 않는다");
        // 보호: 설정 파일은 막힌 목록, 규칙 밖 코드 파일은 AI 제안에도 안 넘긴다
        for f in ["package.json", "tsconfig.json", "docker-compose.yml"] {
            assert!(dest(&format!("다운로드/{f}")).starts_with("[protected]"), "{f}");
        }
        assert_eq!(dest("다운로드/index.ts"), "(없음)");
        assert_eq!(dest("다운로드/main.py"), "(없음)");
        // 두 번 봐도 같다
        assert_eq!(rows, demo_plan(&cfg));
    }

    #[test]
    fn pattern_off_keeps_old_behavior_but_still_protects() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_lang("ko");
        let (mut cfg, _, _) = build();
        cfg.settings.follow_pattern = false;
        let rows = demo_plan(&cfg);
        let dest = |src: &str| rows.iter().find(|(a, _)| a == src).map(|(_, b)| b.clone()).unwrap_or_default();
        assert!(dest("다운로드/VOC Report - raw_3월.csv").starts_with("문서/자료/"), "끄면 규칙 목적지(+자동 소분류)");
        assert!(dest("다운로드/package.json").starts_with("[protected]"), "보호는 끌 수 없다");
    }

    #[test]
    fn direct_moves_cannot_touch_code() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_lang("ko");
        let (cfg, downloads, docs) = build();
        let plan = crate::organizer::plan_from_moves(
            &cfg,
            vec![
                crate::organizer::MoveReq { src: downloads.join("main.py").to_string_lossy().into(), dest_folder: Some(docs.join("자료").to_string_lossy().into()), action: "move".into(), reason: "AI".into() },
                crate::organizer::MoveReq { src: docs.join("my-app/src/index.js").to_string_lossy().into(), dest_folder: Some(docs.join("자료").to_string_lossy().into()), action: "move".into(), reason: "AI".into() },
                crate::organizer::MoveReq { src: downloads.join("계약서.pdf").to_string_lossy().into(), dest_folder: Some(docs.join("웨딩").to_string_lossy().into()), action: "move".into(), reason: "AI".into() },
            ],
        );
        assert_eq!(plan.items.len(), 1, "문서만 계획에 들어간다");
        assert_eq!(plan.rejected.len(), 2);
        assert!(plan.rejected.iter().all(|r| r.reason.starts_with("보호됨")), "{:?}", plan.rejected.iter().map(|r| &r.reason).collect::<Vec<_>>());
    }

    /// 전후 비교 표를 뽑는다: `SMASH_DEMO_OUT=<파일> cargo test --lib demo_pattern -- --ignored`
    #[test]
    #[ignore]
    fn demo_pattern() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_lang("ko");
        let (cfg, _, _) = build();
        let rows = demo_plan(&cfg);
        let text: String = rows.iter().map(|(a, b)| format!("{a}\t{b}\n")).collect();
        if let Ok(out) = std::env::var("SMASH_DEMO_OUT") {
            fs::write(out, &text).unwrap();
        }
        print!("{text}");
    }
}
