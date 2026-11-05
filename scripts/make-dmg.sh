#!/usr/bin/env bash
# Smash.app 을 dmg 로 묶는다. (응용 프로그램 폴더 바로가기 포함)
# Tauri 의 bundle_dmg.sh 는 GitHub 맥 러너에서 "hdiutil: couldn't eject ... Resource busy" 로 자주 실패해서,
# 앱 묶음은 Tauri 로 만들고 dmg 만 여기서 몇 번 다시 시도하며 만든다.
# 사용: scripts/make-dmg.sh <Smash.app 경로> <만들 dmg 경로>
set -euo pipefail
app="$1"
out="$2"
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
cp -R "$app" "$stage/"
ln -s /Applications "$stage/Applications"
rm -f "$out"
for i in 1 2 3 4 5; do
  if hdiutil create -volname "Smash!" -srcfolder "$stage" -fs HFS+ -format UDZO -ov "$out"; then
    hdiutil verify "$out"
    exit 0
  fi
  echo "hdiutil create 실패 ($i/5), 잠시 뒤 다시 시도"
  sleep $((i * 5))
done
echo "dmg 를 만들지 못했습니다" >&2
exit 1
