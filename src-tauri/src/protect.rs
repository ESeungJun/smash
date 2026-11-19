//! 옮기면 안 되는 파일 가리기(#8).
//! 코드·설정 파일과 개발 프로젝트 안의 파일은 규칙·AI 제안·MCP 어느 길로도 계획에 넣지 않는다.
//! 옮기면 프로젝트가 깨지거나, 사용자가 일부러 둔 자리를 흩트리기 때문이다.

use std::path::Path;

use crate::lang::l;
use crate::organizer::{ext_of, PROJECT_MARKERS, PROJECT_MARKER_EXTS};

/// 소스 코드·스크립트 확장자. 규칙이 이 확장자를 직접 적었으면(사용자가 일부러 고른 것) 옮길 수 있다.
const CODE_EXTS: &[&str] = &[
    "js", "mjs", "cjs", "jsx", "ts", "tsx", "mts", "cts", "vue", "svelte", "astro", "py", "pyw", "ipynb", "rb", "go", "rs", "java", "kt", "kts", "scala",
    "groovy", "gradle", "swift", "m", "mm", "h", "hh", "hpp", "c", "cc", "cpp", "cxx", "cs", "fs", "php", "pl", "lua", "r", "dart", "ex", "exs", "erl",
    "clj", "hs", "zig", "sh", "bash", "zsh", "fish", "ps1", "bat", "cmd", "sql", "css", "scss", "sass", "less", "toml", "ini", "cfg", "conf", "env",
    "properties", "xcconfig", "pbxproj", "entitlements", "lock",
];

/// 이름만 봐도 프로젝트·도구 설정인 파일 (소문자로 비교). 어떤 규칙으로도 옮기지 않는다.
const PROJECT_FILES: &[&str] = &[
    "package.json", "package-lock.json", "yarn.lock", "pnpm-lock.yaml", "pnpm-workspace.yaml", "bun.lockb", "tsconfig.json", "jsconfig.json",
    "deno.json", "composer.json", "composer.lock", "cargo.toml", "cargo.lock", "go.mod", "go.sum", "pyproject.toml", "requirements.txt", "pipfile",
    "pipfile.lock", "poetry.lock", "setup.py", "setup.cfg", "gemfile", "gemfile.lock", "podfile", "podfile.lock", "pubspec.yaml", "pubspec.lock",
    "makefile", "cmakelists.txt", "dockerfile", "docker-compose.yml", "docker-compose.yaml", "compose.yml", "compose.yaml", "procfile",
    "build.gradle", "build.gradle.kts", "settings.gradle", "settings.gradle.kts", "gradle.properties", "pom.xml", "androidmanifest.xml",
    "vercel.json", "netlify.toml", "wrangler.toml", "wrangler.json", "firebase.json", ".firebaserc", "app.json", "eas.json", "angular.json",
    "nx.json", "turbo.json", "lerna.json", "biome.json", "claude.md", "agents.md", "gemini.md", ".mcp.json", "settings.json", "settings.local.json",
    "launch.json", "tasks.json", ".editorconfig", ".prettierrc", ".eslintrc", ".eslintrc.json", ".babelrc", ".npmrc", ".nvmrc", ".gitignore",
    ".gitattributes", ".dockerignore",
];

/// 왜 보호했는지
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    /// 프로젝트·도구 설정 파일 이름
    ProjectFile,
    /// 코드·스크립트 확장자
    Code,
    /// 개발 프로젝트 폴더 안
    InProject,
}

impl Reason {
    pub fn text(self) -> String {
        match self {
            Reason::ProjectFile => l("보호됨: 프로젝트 설정 파일", "Protected: project config file"),
            Reason::Code => l("보호됨: 코드 파일", "Protected: code file"),
            Reason::InProject => l("보호됨: 개발 프로젝트 안", "Protected: inside a dev project"),
        }
    }
}

fn is_project_name(lower: &str) -> bool {
    PROJECT_FILES.contains(&lower)
        || lower.starts_with(".env")
        || (lower.starts_with("docker-compose") && (lower.ends_with(".yml") || lower.ends_with(".yaml")))
        || (lower.starts_with("tsconfig") && lower.ends_with(".json"))
        || lower.starts_with("dockerfile")
        // vite.config.js, next.config.mjs, tailwind.config.ts …
        || lower.split('.').rev().nth(1).is_some_and(|s| s == "config") && CODE_EXTS.contains(&ext_of(lower).as_str())
}

/// 이 폴더가 개발 프로젝트인지 (.git, package.json, *.xcodeproj …)
pub fn is_project_dir(dir: &Path) -> bool {
    PROJECT_MARKERS.iter().any(|m| dir.join(m).exists())
        || std::fs::read_dir(dir).is_ok_and(|es| es.flatten().any(|e| PROJECT_MARKER_EXTS.contains(&ext_of(&e.file_name().to_string_lossy()).as_str())))
}

/// 옮기면 안 되는 파일이면 그 이유. `root` 는 정리 대상 폴더: 파일이 있는 폴더부터 root 까지 거슬러 올라가며 프로젝트인지 본다.
/// `allowed_exts`: 규칙이 직접 적은 확장자. 여기에 있는 코드 확장자는 사용자가 일부러 고른 것이라 막지 않는다.
pub fn check(path: &Path, root: &Path, allowed_exts: &[String]) -> Option<Reason> {
    check_with(path, root, allowed_exts, true)
}

/// `walk_up`: false 면 상위 폴더 중 정리 대상 폴더 자체(.git)만 본다. 스캔은 개발 프로젝트 폴더를 이미 빼고 훑어서
/// 파일마다 상위 폴더를 다시 읽을 필요가 없다(하위 폴더까지 정리할 때 수만 번 읽지 않게).
pub fn check_with(path: &Path, root: &Path, allowed_exts: &[String], walk_up: bool) -> Option<Reason> {
    let name = path.file_name()?.to_string_lossy().to_lowercase();
    if is_project_name(&name) {
        return Some(Reason::ProjectFile);
    }
    if !walk_up && root.join(".git").exists() {
        return Some(Reason::InProject);
    }
    let mut dir = if walk_up { path.parent() } else { None };
    while let Some(d) = dir {
        // 정리 대상 폴더 자체는 .git 이 있을 때만 프로젝트로 본다: 다운로드에 package.json 하나가 떨어졌다고
        // 다운로드 전체를 멈추지 않게. 그 아래 폴더는 표식 하나만 있어도 프로젝트다.
        let project = if d == root { d.join(".git").exists() } else { is_project_dir(d) };
        if project {
            return Some(Reason::InProject);
        }
        if d == root || !d.starts_with(root) {
            break;
        }
        dir = d.parent();
    }
    let ext = ext_of(&name);
    if CODE_EXTS.contains(&ext.as_str()) && !allowed_exts.iter().any(|e| e.eq_ignore_ascii_case(&ext)) {
        return Some(Reason::Code);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{sandbox, write};

    #[test]
    fn code_and_project_files_are_protected() {
        let root = sandbox().join("protect");
        let _ = std::fs::remove_dir_all(&root);
        for f in ["package.json", "tsconfig.app.json", "docker-compose.dev.yml", ".env.local", "vite.config.ts", "index.ts", "main.py", "notes.md", "logo.svg", "data.json", "app/README.md", "app/Cargo.toml"] {
            write(&root.join(f), "x", 0);
        }
        let c = |f: &str| check(&root.join(f), &root, &[]);
        assert_eq!(c("package.json"), Some(Reason::ProjectFile));
        assert_eq!(c("tsconfig.app.json"), Some(Reason::ProjectFile));
        assert_eq!(c("docker-compose.dev.yml"), Some(Reason::ProjectFile));
        assert_eq!(c(".env.local"), Some(Reason::ProjectFile));
        assert_eq!(c("vite.config.ts"), Some(Reason::ProjectFile));
        assert_eq!(c("index.ts"), Some(Reason::Code));
        assert_eq!(c("main.py"), Some(Reason::Code));
        assert_eq!(c("notes.md"), None, "문서는 막지 않는다");
        assert_eq!(c("logo.svg"), None);
        assert_eq!(c("data.json"), None, "흔한 json 은 막지 않는다(리소스일 수 있다)");
        assert_eq!(c("app/README.md"), Some(Reason::InProject), "프로젝트 폴더 안이면 무엇이든 막는다");
        assert_eq!(check(&root.join("main.py"), &root, &["py".into()]), None, "규칙이 py 를 직접 적었으면 사용자가 고른 것");
        assert_eq!(check(&root.join("package.json"), &root, &["json".into()]), Some(Reason::ProjectFile), "설정 파일 이름은 늘 막는다");
    }

    #[test]
    fn root_is_a_project_only_with_git() {
        let root = sandbox().join("protect-proj");
        let _ = std::fs::remove_dir_all(&root);
        write(&root.join("Cargo.toml"), "x", 0);
        write(&root.join("photo.jpg"), "x", 0);
        assert_eq!(check(&root.join("photo.jpg"), &root, &[]), None, "다운로드에 떨어진 설정 파일 하나로 폴더 전체를 멈추지 않는다");
        write(&root.join(".git").join("HEAD"), "x", 0);
        assert_eq!(check(&root.join("photo.jpg"), &root, &[]), Some(Reason::InProject), ".git 이 있으면 정리 대상 자체가 저장소");
    }
}
