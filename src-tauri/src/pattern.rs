//! 기존 정리 패턴 따르기(#8).
//! 규칙의 목적지가 들어 있는 정리 대상 폴더(보통 문서 폴더)를 훑어, 사용자가 이미 만들어 둔 폴더와
//! 그 안 파일 이름·확장자를 색인한다. 새 파일은 아래 순서로 처음 근거가 분명한 곳에 놓는다.
//!
//!  1. 폴더 이름: 파일 이름에 기존 폴더 이름의 낱말이 들어 있다 (웨딩드레스_견적.pdf → 웨딩/)
//!  2. 비슷한 파일: 이름의 특징 낱말을 같이 쓰는 파일이 한 폴더에 모여 있다 (VOC Report 3월 → VOC Report 1·2월이 있는 곳)
//!  3. 같은 확장자가 모인 곳: 규칙 목적지 안에서 그 확장자 대부분이 한 하위 폴더에 있다 (.otf → 개발리소스/폰트)
//!  4. 같은 확장자가 모인 곳(전체): 규칙 목적지에는 그 확장자가 하나도 없고, 다른 한 폴더에 80% 넘게 모여 있다 (.svg → 아이콘·에셋)
//!  5. 아니면 None: 규칙 목적지 그대로
//!
//! 근거가 애매하면(두 폴더가 비슷하게 맞으면) 고르지 않는다. 고른 곳은 늘 이미 있는 폴더라 새 폴더를 만들지 않고,
//! 같은 폴더 상태에서는 늘 같은 결과가 나온다. 파일 내용은 보지 않는다.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use crate::config::CATEGORIES;
use crate::grouping::{name_has, PURPOSES, STOP_WORDS};
use crate::lang::l;
use crate::organizer::{ext_of, is_opaque_dir, nfc, Skipper};

const MAX_DEPTH: usize = 6;
const MAX_FILES: usize = 8_000;
const MAX_FOLDERS: usize = 600;
/// 폴더 하나에서 보는 항목 수. 하위 폴더가 수만 개인 폴더에서 멈추지 않게.
const MAX_ENTRIES: usize = 2_000;

struct Folder {
    path: PathBuf,
    depth: usize,
    /// 폴더 이름의 낱말과, 그 낱말이 용도 낱말(스크린샷·영수증 …)인지
    words: Vec<(String, bool)>,
    /// 바로 아래 파일 이름 낱말 → 그 낱말이 들어간 파일 수
    tokens: HashMap<String, usize>,
    /// 바로 아래 파일 확장자 → 개수
    exts: HashMap<String, usize>,
    /// 하위 폴더까지 합친 확장자·종류 (종류가 맞는지 볼 때)
    tree_exts: HashSet<String>,
    tree_cats: HashSet<&'static str>,
    /// 하위 폴더까지 합친 확장자 → 개수
    tree_counts: HashMap<String, usize>,
}

/// 한 정리 대상 폴더의 색인
pub struct Index {
    root: PathBuf,
    folders: Vec<Folder>,
    /// 낱말 → 그 낱말을 가진 폴더 수 (흔한 낱말일수록 근거가 약하다)
    df: HashMap<String, usize>,
    /// 폴더 이름 낱말 → 그 낱말을 이름에 가진 폴더 수
    name_df: HashMap<String, usize>,
}

/// 고른 곳과 이유
#[derive(Debug, Clone, PartialEq)]
pub struct Pick {
    pub dir: PathBuf,
    pub why: String,
}

fn category_of(ext: &str) -> Option<&'static str> {
    CATEGORIES.iter().find(|c| c.extensions.contains(&ext)).map(|c| c.key)
}

fn is_generic(t: &str) -> bool {
    STOP_WORDS.contains(&t) || PURPOSES.iter().any(|p| p.words.contains(&t))
}

/// 이름의 낱말: 글자·숫자가 아닌 것으로 자르고, 숫자가 섞인 낱말(날짜·번호)과 짧은 영문(3자 미만), 흔한 낱말은 뺀다.
pub fn tokens(name: &str) -> Vec<String> {
    split_words(name, true)
}

/// 폴더 이름의 낱말. 용도 낱말도 남긴다(사용자가 그 이름으로 폴더를 만들었으면 그게 분류다).
/// 확장자·종류 이름(pdf, png, docs …)은 폴더 이름 낱말로 쓰지 않는다: `PDF/` 폴더 하나가 모든 PDF 를 끌어가지 않게.
fn folder_words(name: &str) -> Vec<(String, bool)> {
    const KIND_WORDS: &[&str] = &["docs", "documents", "files", "images", "image", "photos", "videos", "video", "audio", "music", "fonts", "font"];
    split_words(&format!("{name}.d"), false)
        .into_iter()
        .filter(|w| !KIND_WORDS.contains(&w.as_str()) && !CATEGORIES.iter().any(|c| c.extensions.contains(&w.as_str())))
        .map(|w| {
            let g = is_generic(&w);
            (w, g)
        })
        .collect()
}

/// 이름(확장자 뺀)에 폴더 낱말이 있는지. 영문은 낱말 경계로만 맞춘다(`work` 가 `homework` 를 끌어가지 않게).
fn stem_has(stem: &str, word: &str) -> bool {
    if !word.is_ascii() {
        return name_has(stem, word);
    }
    stem.match_indices(word).any(|(i, _)| {
        let before = stem[..i].chars().next_back();
        let after = stem[i + word.len()..].chars().next();
        !before.is_some_and(|c| c.is_ascii_alphanumeric()) && !after.is_some_and(|c| c.is_ascii_alphanumeric())
    })
}

fn split_words(name: &str, drop_generic: bool) -> Vec<String> {
    let stem = Path::new(name).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| name.to_string());
    let mut out: Vec<String> = Vec::new();
    for t in nfc(&stem).to_lowercase().split(|c: char| !c.is_alphanumeric()) {
        let n = t.chars().count();
        if n < 2 || t.chars().any(|c| c.is_ascii_digit()) || (t.is_ascii() && n < 3) || STOP_WORDS.contains(&t) || (drop_generic && is_generic(t)) || out.iter().any(|o| o == t) {
            continue;
        }
        out.push(t.to_string());
    }
    out
}

/// 자동 정리는 파일이 다 받아질 때까지 몇 초마다 다시 스캔한다. 그때마다 문서 폴더를 처음부터 훑지 않게 잠깐 들고 있는다.
#[cfg(not(test))]
const CACHE_SECS: u64 = 15;

impl Index {
    /// `build` 와 같지만 같은 폴더·같은 제외 목록이면 잠깐(15초) 전에 만든 색인을 다시 쓴다. 시험에서는 늘 새로 만든다.
    pub fn cached(root: &Path, skip: &Skipper, exclude: &[PathBuf]) -> std::sync::Arc<Index> {
        #[cfg(test)]
        {
            std::sync::Arc::new(Index::build(root, skip, exclude))
        }
        #[cfg(not(test))]
        {
            use std::sync::{Arc, Mutex, OnceLock};
            use std::time::{Duration, Instant};
            type Slot = (PathBuf, Vec<PathBuf>, Instant, Arc<Index>);
            static CACHE: OnceLock<Mutex<Vec<Slot>>> = OnceLock::new();
            let cache = CACHE.get_or_init(|| Mutex::new(Vec::new()));
            let mut slots = cache.lock().unwrap_or_else(|e| e.into_inner());
            slots.retain(|s| s.2.elapsed() < Duration::from_secs(CACHE_SECS));
            if let Some(hit) = slots.iter().find(|s| s.0 == root && s.1 == exclude) {
                return hit.3.clone();
            }
            let idx = Arc::new(Index::build(root, skip, exclude));
            slots.push((root.to_path_buf(), exclude.to_vec(), Instant::now(), idx.clone()));
            idx
        }
    }

    /// `root` 아래 폴더를 훑는다(깊이 6, 파일 8,000개, 폴더 600개, 폴더마다 항목 2,000개까지). 숨김·건너뛰기 폴더, 앱 꾸러미, 개발 프로젝트,
    /// `exclude`(다른 정리 대상 폴더)는 들어가지 않는다. root 바로 아래 파일은 "아직 정리 안 된 것"이라 배우지 않는다.
    pub fn build(root: &Path, skip: &Skipper, exclude: &[PathBuf]) -> Index {
        let mut folders: Vec<Folder> = Vec::new();
        let mut seen_files = 0usize;
        // 다른 정리 대상 폴더는 링크·대소문자가 달라도 빼도록 실제 경로로도 비교한다
        let exclude_real: Vec<PathBuf> = exclude.iter().filter_map(|p| p.canonicalize().ok()).collect();
        let excluded = |p: &Path| exclude.iter().any(|x| x == p) || p.canonicalize().is_ok_and(|r| exclude_real.contains(&r));
        let mut stack: Vec<(PathBuf, usize)> = vec![(root.to_path_buf(), 0)];
        while let Some((dir, depth)) = stack.pop() {
            let Ok(entries) = fs::read_dir(&dir) else { continue };
            let mut subdirs = Vec::new();
            let mut f = Folder {
                path: dir.clone(),
                depth,
                words: if depth == 0 { vec![] } else { folder_words(&dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()) },
                tokens: HashMap::new(),
                exts: HashMap::new(),
                tree_exts: HashSet::new(),
                tree_cats: HashSet::new(),
                tree_counts: HashMap::new(),
            };
            // 이름순으로 보고 상한을 둔다: 상한에 닿아도 같은 폴더 상태면 같은 결과
            let mut entries: Vec<fs::DirEntry> = entries.flatten().take(MAX_ENTRIES * 4).collect();
            entries.sort_by_key(|e| e.file_name());
            for e in entries.into_iter().take(MAX_ENTRIES) {
                let name = e.file_name().to_string_lossy().into_owned();
                let Ok(ft) = e.file_type() else { continue };
                if skip.skip(&name) {
                    continue;
                }
                if ft.is_dir() {
                    let p = e.path();
                    if depth < MAX_DEPTH && !is_opaque_dir(&p, &name) && !excluded(&p) {
                        subdirs.push(p);
                    }
                } else if ft.is_file() && depth > 0 && seen_files < MAX_FILES {
                    seen_files += 1;
                    let ext = ext_of(&name);
                    *f.exts.entry(ext.clone()).or_default() += 1;
                    for t in tokens(&name) {
                        *f.tokens.entry(t).or_default() += 1;
                    }
                }
            }
            subdirs.sort();
            for s in subdirs.into_iter().rev() {
                stack.push((s, depth + 1));
            }
            folders.push(f);
            if folders.len() >= MAX_FOLDERS {
                break;
            }
        }
        // 하위 폴더까지 합친 종류: 깊은 폴더부터 위로 올린다
        let mut order: Vec<usize> = (0..folders.len()).collect();
        order.sort_by_key(|&i| std::cmp::Reverse(folders[i].depth));
        for i in order {
            let own: Vec<(String, usize)> = folders[i].exts.iter().map(|(e, n)| (e.clone(), *n)).collect();
            for (e, n) in &own {
                *folders[i].tree_counts.entry(e.clone()).or_default() += n;
            }
            folders[i].tree_exts.extend(own.into_iter().map(|(e, _)| e));
            let counts: Vec<(String, usize)> = folders[i].tree_counts.iter().map(|(e, n)| (e.clone(), *n)).collect();
            let (exts, cats): (Vec<String>, Vec<&'static str>) =
                (folders[i].tree_exts.iter().cloned().collect(), folders[i].tree_exts.iter().filter_map(|e| category_of(e)).collect());
            folders[i].tree_cats.extend(cats.iter().copied());
            if let Some(parent) = folders[i].path.parent().map(Path::to_path_buf) {
                if let Some(p) = folders.iter_mut().find(|f| f.path == parent) {
                    p.tree_exts.extend(exts);
                    p.tree_cats.extend(cats);
                    for (e, n) in counts {
                        *p.tree_counts.entry(e).or_default() += n;
                    }
                }
            }
        }
        let mut df: HashMap<String, usize> = HashMap::new();
        let mut name_df: HashMap<String, usize> = HashMap::new();
        for f in &folders {
            for t in f.tokens.keys() {
                *df.entry(t.clone()).or_default() += 1;
            }
            // 조상 폴더가 같은 낱말을 이름에 가지면 한 갈래로 본다 (이력서/01_이력서 는 하나)
            for (w, _) in &f.words {
                let ancestor_has = folders.iter().any(|a| a.path != f.path && f.path.starts_with(&a.path) && a.words.iter().any(|(x, _)| x == w));
                if !ancestor_has {
                    *name_df.entry(w.clone()).or_default() += 1;
                }
            }
        }
        Index { root: root.to_path_buf(), folders, df, name_df }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn rel(&self, p: &Path) -> String {
        p.strip_prefix(&self.root).map(|r| r.to_string_lossy().replace('\\', "/")).unwrap_or_else(|_| p.to_string_lossy().into_owned())
    }

    /// 그 폴더(하위 포함)에 이 파일과 같은 확장자나 같은 종류가 있는지. 빈 폴더는 사용자가 만들어 둔 자리라 받아 준다.
    fn fits(&self, f: &Folder, ext: &str) -> bool {
        f.tree_exts.is_empty() || f.tree_exts.contains(ext) || category_of(ext).is_some_and(|c| f.tree_cats.contains(c))
    }

    /// 새 파일을 놓을 기존 폴더. `rule_dest` 는 규칙이 정한 목적지(색인 root 안이어야 한다).
    pub fn place(&self, name: &str, rule_dest: &Path) -> Option<Pick> {
        let ext = ext_of(name);
        let stem = Path::new(name).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| name.to_string());
        let lower = nfc(&stem).to_lowercase();
        let inside_dest = |f: &Folder| f.path.starts_with(rule_dest);
        let candidates: Vec<&Folder> = self.folders.iter().filter(|f| f.depth > 0 && self.fits(f, &ext)).collect();

        // 1. 폴더 이름 낱말 (여러 갈래 폴더 이름에 쓰인 낱말은 근거가 못 된다).
        //    용도 낱말(스크린샷 …) 이름은 맨 위 폴더나 규칙 목적지 안일 때만: 다른 주제 폴더 깊숙한 곳(이력서/04_스크린샷)으로 빨려 들어가지 않게
        let mut named: Vec<(&Folder, usize)> = candidates
            .iter()
            .map(|f| {
                let ok_generic = f.depth == 1 || inside_dest(f);
                (*f, f.words.iter().filter(|(w, g)| (!g || ok_generic) && self.name_df.get(w).copied().unwrap_or(0) <= 1 && stem_has(&lower, w)).count())
            })
            .filter(|(_, n)| *n > 0)
            .collect();
        named.sort_by(|a, b| b.1.cmp(&a.1).then(b.0.depth.cmp(&a.0.depth)));
        if let Some(&(best, n)) = named.first() {
            // 같은 수로 맞은 다른 폴더가 best 의 조상이 아니면 애매하다
            let clash = named.iter().skip(1).any(|(f, m)| *m == n && !best.path.starts_with(&f.path));
            if !clash {
                return Some(Pick { dir: best.path.clone(), why: l(&format!("폴더 이름이 같아서 {}", self.rel(&best.path)), &format!("matches folder name {}", self.rel(&best.path))) });
            }
        }

        // 2. 비슷한 파일이 모인 곳
        let toks = tokens(name);
        let mut scored: Vec<(&Folder, f64, usize)> = Vec::new();
        for f in &candidates {
            let mut score = 0.0;
            let mut hits = 0;
            for t in &toks {
                let Some(&count) = f.tokens.get(t) else { continue };
                let idf = match self.df.get(t).copied().unwrap_or(0) {
                    1 => 1.0,
                    2 => 0.5,
                    _ => 0.0,
                };
                let strength = if count >= 2 || t.chars().count() >= 5 { 1.0 } else { 0.5 };
                score += idf * strength;
                hits = hits.max(count);
            }
            if score > 0.0 {
                scored.push((*f, score, hits));
            }
        }
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        if let Some(&(best, score, hits)) = scored.first() {
            let second = scored.get(1).map(|s| s.1).unwrap_or(0.0);
            let need = if inside_dest(best) { 0.5 } else { 1.0 };
            if score >= need && score >= second * 2.0 {
                return Some(Pick {
                    dir: best.path.clone(),
                    why: l(&format!("비슷한 파일 {hits}개가 있는 {}", self.rel(&best.path)), &format!("{hits} similar files in {}", self.rel(&best.path))),
                });
            }
        }

        if ext.is_empty() {
            return None;
        }
        // 3·4. 같은 확장자가 모인 곳. 하위 폴더까지 합쳐 센다(폰트/Pretendard-1.3.9/… 처럼 깊이 있어도)
        let tree_n = |f: &Folder| f.tree_counts.get(&ext).copied().unwrap_or(0);
        let dest_total = self.folders.iter().find(|f| f.path == rule_dest).map(tree_n).unwrap_or(0);
        let say = |f: &Folder, n: usize| l(&format!(".{ext} 파일 {n}개가 모인 {}", self.rel(&f.path)), &format!("{n} .{ext} files live in {}", self.rel(&f.path)));
        // 3. 규칙 목적지 바로 아래 분류 폴더 중 그 확장자 60% 이상을 가진 곳 (꾸러미 속 깊은 폴더가 아니라 분류 폴더로)
        if dest_total >= 3 {
            let best = self.folders.iter().filter(|f| f.path.parent() == Some(rule_dest)).max_by_key(|f| tree_n(f));
            if let Some(best) = best.filter(|f| tree_n(f) as f64 >= dest_total as f64 * 0.6) {
                return Some(Pick { dir: best.path.clone(), why: say(best, tree_n(best)) });
            }
        }
        // 4. 규칙 목적지엔 하나도 없고, 맨 위 두 단계 폴더 중 한 곳에 80% 넘게 모여 있으면 그중 가장 안쪽
        // 규칙 목적지가 아직 없으면(처음 쓰는 사용자) 다른 폴더로 돌리지 않고 규칙대로 둔다
        if dest_total == 0 && rule_dest.is_dir() {
            let all: usize = self.folders.iter().filter(|f| f.depth == 1).map(tree_n).sum();
            let best = self
                .folders
                .iter()
                .filter(|f| (1..=2).contains(&f.depth) && tree_n(f) >= 5 && tree_n(f) as f64 >= all as f64 * 0.8)
                .max_by_key(|f| (f.depth, tree_n(f)));
            if let Some(best) = best {
                return Some(Pick { dir: best.path.clone(), why: say(best, tree_n(best)) });
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_skip_dates_short_and_common_words() {
        assert_eq!(tokens("VOC Report - raw_3월.csv"), vec!["voc", "raw"], "report 는 흔한 낱말, 3월은 숫자가 섞임");
        assert_eq!(tokens("2026_원천징수영수증.pdf"), vec!["원천징수영수증"]);
        assert_eq!(tokens("스크린샷 2026-11-19 오후 3.00.00.png"), Vec::<String>::new());
        assert_eq!(tokens("ic_search.svg"), vec!["search"]);
    }
}

#[cfg(test)]
mod real_tree {
    /// 실제 폴더에 대고 읽기만 하는 확인: `SMASH_REAL_ROOT=~/Documents cargo test --lib real_tree -- --ignored --nocapture`
    /// 파일은 옮기지 않는다. 이름 목록은 SMASH_REAL_NAMES(줄마다 "이름\t규칙 목적지 상대 경로").
    #[test]
    #[ignore]
    fn place_against_real_tree() {
        let Ok(root) = std::env::var("SMASH_REAL_ROOT") else { return };
        let root = std::path::PathBuf::from(root);
        let cfg = crate::config::default_config();
        let idx = super::Index::build(&root, &crate::organizer::Skipper::new(&cfg), &[]);
        for line in std::env::var("SMASH_REAL_NAMES").unwrap_or_default().lines() {
            let (name, dest) = line.split_once('\t').unwrap_or((line, ""));
            let got = idx.place(name, &root.join(dest));
            println!("{name}\t→ {}", got.map(|p| format!("{} ({})", idx.rel(&p.dir), p.why)).unwrap_or_else(|| format!("{dest} (규칙 목적지)")));
        }
    }
}
