'use strict';
/* global tidy */
// 설정 탭: 자동 정리·효과·알림·언어 카드, 자동 정리를 켜기 전 확인, Claude 연결 상태(사이드바 알약 포함)와 MCP 등록,
// 앱 정보·업데이트 카드를 맡는다. events.js(스위치·버튼·업데이트 알림)와 app.js(시작·탭 전환)가 window.SmashSettings 로 부른다.
(function (root) {
  const { $, L, esc, toast, ico, switchHtml, dotLine } = root.SmashUI;
  const { state, saveConfig, dirOf, targetName, cliReady, destText } = root.SmashState;

  function setPill() {
    const s = state.integration;
    if (!s) return;
    const linked = s.claudeCode || s.claudeDesktop;
    $('#claude-dot').className = `dot ${cliReady() || linked ? 'on' : 'off'}`;
    $('#claude-pill-text').textContent = linked ? L('Claude와 연결됨', 'Connected to Claude') : cliReady() ? L('Claude CLI 사용 가능', 'Claude CLI available') : L('Claude 연결 안 됨', 'Claude not connected');
  }

  async function refreshIntegration() {
    try {
      state.integration = await tidy.integrationStatus();
    } catch (err) {
      state.integration = { cli: { installed: false, error: err.message }, locations: [], claudeCode: false, claudeDesktop: false, launch: null };
    }
    setPill();
  }

  // 설정 탭 맨 아래 앱 정보 카드. 업데이트 상태가 바뀌면 이 카드만 다시 그린다.
  function updateCardHtml() {
    const u = state.update;
    if (!u) return '';
    let status;
    let button = '';
    if (u.ready) {
      status = dotLine(true, L(`새 버전(v${esc(u.latest)})을 받아 뒀어요`, `v${esc(u.latest)} is downloaded and ready`), '');
      button = `<button class="primary small" id="update-install">${L('설치하고 다시 시작', 'Install and restart')}</button>`;
    } else if (u.checking) {
      status = `<div class="status-line muted"><span class="spinner"></span>${
        u.latest ? L(`새 버전 v${esc(u.latest)} 받는 중…`, `Downloading v${esc(u.latest)}…`) : L('업데이트 확인 중…', 'Checking for updates…')
      }</div>`;
    } else if (u.error) {
      status = dotLine(false, '', L('업데이트를 확인하지 못했어요. 인터넷 연결을 확인해 주세요.', "Couldn't check for updates. Check your internet connection."));
      button = `<button class="ghost small" id="update-check">${L('다시 확인', 'Try again')}</button>`;
    } else {
      status = u.checked ? dotLine(true, L('최신 버전이에요', "You're up to date"), '') : '';
      button = `<button class="ghost small" id="update-check">${L('업데이트 확인', 'Check for updates')}</button>`;
    }
    return `<div class="card" id="update-card">
    <h3>Smash! <span class="muted">v${esc(u.current)}</span></h3>
    <p>${L('새 버전은 저절로 받아 둬요. 설치하면 앱이 잠깐 꺼졌다가 다시 켜져요.', 'New versions download on their own. Installing restarts the app.')}</p>
    ${status}
    <div class="card-actions">${button}<button class="ghost small" data-url="https://github.com/ESeungJun/smash/releases">${L('릴리스 노트', 'Release notes')}</button></div>
  </div>`;
  }

  function renderUpdate() {
    const card = $('#update-card');
    if (card) card.outerHTML = updateCardHtml();
  }

  async function checkUpdate() {
    state.update = { ...state.update, checking: true, error: false };
    renderUpdate();
    try {
      state.update = await tidy.checkUpdate();
    } catch {
      state.update = { ...state.update, checking: false, error: true };
    }
    renderUpdate();
  }

  function renderSettings() {
    const cfg = state.config;
    const st = cfg.settings;
    const delays = [
      [5, L('5초', '5 sec')],
      [10, L('10초', '10 sec')],
      [30, L('30초', '30 sec')],
      [60, L('1분', '1 min')],
      [300, L('5분', '5 min')],
    ];
    const autoCard = `<div class="card">
    <h3>${L('자동 정리', 'Auto-tidy')}</h3>
    <p>${L(
        `켜 둔 폴더에 새 파일이 들어오면, 다 받아질 때까지 기다렸다가 규칙대로 정리해요. 창을 닫아도 ${
          state.platform === 'darwin' ? '메뉴 막대' : '작업 표시줄 알림 영역'
        }에서 계속 지켜봐요. AI 제안은 쓰지 않고 내가 만든 규칙만 써요.`,
        `When a new file lands in a folder you turned on, Smash! waits until it finishes downloading, then sorts it by your rules. It keeps watching from the ${
          state.platform === 'darwin' ? 'menu bar' : 'notification area'
        } after you close the window. Only your own rules are used, never AI picks.`,
      )}</p>
    <div class="settings-list">
      ${cfg.targets
          .map(
            (t) => `<div class="setting-row"><div><b>${esc(targetName(t))}</b><div class="muted">${esc(t.path)} · ${L(`켜진 규칙 ${t.rules.filter((r) => r.enabled).length}개`, `${t.rules.filter((r) => r.enabled).length} rules on`)}${
              t.auto && t.autoTrash === false ? ` · ${L('휴지통 규칙은 자동으로 안 해요', 'Trash rules stay manual')}` : ''
            }</div></div>${switchHtml(`data-auto="${esc(t.id)}"`, t.auto)}</div>${state.autoConfirm?.id === t.id ? autoConfirmHtml(t) : ''}`,
          )
          .join('')}
      <div class="setting-row"><div><b>${L('기다리는 시간', 'Wait time')}</b><div class="muted">${L('파일이 이만큼 그대로면 다 받아진 걸로 봐요', 'A file that stays the same this long counts as downloaded')}</div></div>
        <select id="auto-delay">${delays.map(([v, l]) => `<option value="${v}" ${st.autoDelaySec === v ? 'selected' : ''}>${l}</option>`).join('')}</select></div>
      <div class="setting-row"><div><b>${L('정리할 때 등짝 스매시 효과', 'Smash effect when tidying')}</b><div class="muted">${L('정리하기를 누르면 손바닥이 "찰싹!!" 내리쳐요. 자동 정리 때는 안 나와요', 'A palm comes down with a "SMACK!!" when you press tidy. Not shown for auto-tidy')}</div></div>${switchHtml('id="set-smash"', st.smash)}</div>
      <div class="setting-row"><div><b>${L('정리할 때 소리', 'Sound when tidying')}</b><div class="muted">${L('손바닥이 닿는 순간 "찰싹" 소리가 나요. 파일이 많을수록 세게 쳐요. 자동 정리 때는 안 나요', 'A smack sound at the moment of impact, harder the more files there are. Silent for auto-tidy')}</div></div>${switchHtml('id="set-sound"', st.sound !== false)}</div>
      <div class="setting-row"><div><b>${L('쌓이면 잔소리하기', 'Nag me when files pile up')}</b><div class="muted">${L('자동 정리를 안 켠 폴더에 치울 파일이 20개·60개·150개 쌓이면 배너로 한 번씩 알려요. 폴더마다 하루 한 번까지', 'When a folder without auto-tidy piles up 20, 60 or 150 files to tidy, a banner tells you once. At most once a day per folder')}</div></div>${switchHtml('id="set-nag"', st.nag !== false)}</div>
      <div class="setting-row"><div><b>${L('일요일 저녁 주간 성적표', 'Weekly report on Sunday evening')}</b><div class="muted">${L('이번 주에 몇 개 치웠는지 배너로 한 번 알려요', 'One banner with how much was tidied this week')}</div></div>${switchHtml('id="set-weekly"', st.weekly !== false)}</div>
      <div class="setting-row"><div><b>${L('정리한 뒤 폴더 열기', 'Open the folder afterwards')}</b><div class="muted">${L(
          `정리하기를 누르면 파일이 들어간 폴더를 ${state.platform === 'darwin' ? 'Finder' : '탐색기'}로 열어 줘요`,
          `Opens the folder the files went to in ${state.platform === 'darwin' ? 'Finder' : 'Explorer'}`,
        )}</div></div>${switchHtml('id="set-open"', st.openAfter)}</div>
      <div class="setting-row"><div><b>${L('정리하면 알림 보내기', 'Notify me after tidying')}</b><div class="muted">${L('끄면 잔소리와 주간 성적표 배너도 안 떠요', 'Turning this off also silences the nag and weekly banners')}</div></div>${switchHtml('id="set-notify"', st.notify)}</div>
      <div class="setting-row"><div><b>${L('컴퓨터 켤 때 같이 실행', 'Open at login')}</b><div class="muted">${L('창 없이 조용히 시작해요 (설치한 앱에서만 동작)', 'Starts quietly without a window (installed app only)')}</div></div>${switchHtml('id="set-login"', st.openAtLogin)}</div>
      <div class="setting-row"><div><b>언어 · Language</b><div class="muted">${L('바꾸면 화면을 다시 불러와요', 'The window reloads when you change it')}</div></div>
        <select id="set-lang">${[
            ['system', L('시스템 설정 따르기', 'Same as system')],
            ['ko', '한국어'],
            ['en', 'English'],
          ]
            .map(([v, l]) => `<option value="${v}" ${(st.language || 'system') === v ? 'selected' : ''}>${l}</option>`)
            .join('')}</select></div>
    </div>
  </div>`;

    const s = state.integration;
    if (!s) {
      $('#settings').innerHTML = `${autoCard}<div class="card"><span class="spinner"></span>${L('Claude 연결 확인 중…', 'Checking Claude…')}</div>${updateCardHtml()}`;
      return;
    }
    const cli = s.cli;
    const model = cfg.ai?.model || 'sonnet';
    const shellQuote = (v) => (/[\s"'$`\\]/.test(v) ? `"${v.replace(/(["\\$`])/g, '\\$1')}"` : v);
    const cmd = s.launch
      ? `claude mcp add -s user ${s.mcpName}${Object.entries(s.launch.env || {}).map(([k, v]) => ` -e ${k}=${shellQuote(v)}`).join('')} -- ${shellQuote(s.launch.command)} ${s.launch.args.map(shellQuote).join(' ')}`
      : '';
    $('#settings').innerHTML = `${autoCard}
    <div class="card">
      <h3>${L('Claude 연결', 'Claude')} <span class="muted">${L('API 키 없이', 'no API key')}</span></h3>
      <p>${L('내 PC에 로그인된 Claude를 그대로 빌려 써요. 사용량은 내 Claude 구독에 포함돼요.', 'Uses the Claude you are signed in to on this computer. Usage counts toward your Claude plan.')}</p>
      ${
          cli.installed
            ? dotLine(!cli.error, `Claude Code CLI ${esc(cli.version || '')} <span class="muted">${esc(cli.path)}</span>`, L(`CLI 실행 오류: ${esc(cli.error)}`, `CLI error: ${esc(cli.error)}`))
            : dotLine(false, '', L('Claude Code CLI가 없어요', 'Claude Code CLI not found'))
        }
      <div class="card-actions">
        ${cli.installed ? '' : `<button class="primary small" data-url="https://claude.com/claude-code">${L('설치 방법 보기', 'How to install')}</button>`}
        <label class="muted">${L('자리 제안·말로 규칙 만들기에 쓸 모델', 'Model for suggestions and described rules')}
          <select id="ai-model">${['haiku', 'sonnet', 'opus'].map((m) => `<option value="${m}" ${m === model ? 'selected' : ''}>${m}</option>`).join('')}</select>
        </label>
      </div>
      <div class="setting-row vision-row">
        <div><b>${L('사진·동영상 내용으로 묶기', 'Group photos & videos by content')}</b><div class="muted">${L(
            '켜면 미리보기에 "내용 보고 묶기" 버튼이 생겨요. 누를 때만 줄인 썸네일(긴 변 384px, 동영상은 한 장면)과 파일 이름, 이미 있는 하위 폴더 이름을 Claude에 보내서 게임·음식·풍경처럼 비슷한 것끼리 하위 폴더로 묶어요. 원본 파일은 보내지 않아요.',
            'Adds a "Group by content" button to the preview. Only when you press it, small thumbnails (384px, one frame per video), file names and existing subfolder names go to Claude to group similar ones (games, food, scenery). Original files are never sent.',
          )}</div></div>
        ${switchHtml('id="set-vision"', !!cfg.ai?.vision)}
      </div>
    </div>

    <div class="card">
      <h3>${L('Claude Code에 MCP 서버로 등록', 'Add as an MCP server to Claude Code')}</h3>
      <p>${L(
          '등록하면 Claude Code에서 “다운로드 정리해줘”라고 말할 때 이 앱의 규칙·기록을 그대로 써요. 찾은 Claude 설정 위치 모두에 등록해요.',
          'Then when you ask Claude Code to “tidy my Downloads”, it uses this app’s rules and history. Added to every Claude config location found.',
        )}</p>
      <div class="loc-list">${(s.locations || [])
          .map((l) => `<div class="loc-row"><span class="dot ${l.registered ? 'on' : 'off'}"></span><span>${esc(l.label)}</span><span class="muted">${l.registered ? L('등록됨', 'Added') : L('등록 안 됨', 'Not added')}</span></div>`)
          .join('')}</div>
      <div class="card-actions">
        <button class="primary small" id="reg-code" ${cliReady() ? '' : 'disabled'}>${s.claudeCode ? L('다시 등록', 'Add again') : L('모두 등록하기', 'Add to all')}</button>
        <span class="muted">${L('등록 후 새로 연 Claude Code 세션부터 보여요.', 'Shows up in Claude Code sessions opened after this.')}</span>
      </div>
      <details class="cmd-details"><summary>${L('직접 등록하는 명령 보기', 'Show the command to add it yourself')}</summary><pre class="cmd">${esc(cmd)}</pre></details>
    </div>

    <div class="card">
      <h3>${L('Claude Desktop에 MCP 서버로 등록', 'Add as an MCP server to Claude Desktop')}</h3>
      <p>${L(
          `Claude Desktop 설정 파일(${esc(s.desktopConfig)})에 추가해요. 기존 파일은 .bak으로 백업하고, 추가한 뒤 Claude Desktop을 다시 시작해야 해요.`,
          `Adds it to the Claude Desktop config (${esc(s.desktopConfig)}). The old file is backed up as .bak. Restart Claude Desktop afterwards.`,
        )}</p>
      ${dotLine(s.claudeDesktop, L('등록됨', 'Added'), s.desktopInstalled ? L('등록 안 됨', 'Not added') : L('Claude Desktop이 설치돼 있지 않아요', 'Claude Desktop is not installed'))}
      <div class="card-actions"><button class="primary small" id="reg-desktop" ${s.desktopInstalled ? '' : 'disabled'}>${s.claudeDesktop ? L('다시 등록', 'Add again') : L('등록하기', 'Add')}</button></div>
    </div>
    ${updateCardHtml()}
    <p class="hint">${L('설정 파일', 'Settings file')}: ${esc(state.configPath)}</p>`;
  }

  // 자동 정리를 켜기 전에, 지금 쌓여 있는 파일을 어떻게 치울지 한 번 보여 준다. 켜는 순간 말없이 치우지 않는다.
  function autoConfirmHtml(t) {
    const c = state.autoConfirm;
    if (!c.scan) return `<div class="auto-confirm"><span class="spinner"></span>${L('지금 쌓인 파일을 살펴보는 중…', 'Looking at what is there now…')}</div>`;
    const all = c.scan.items;
    const trash = all.filter((it) => it.action === 'trash' && !it.note);
    const items = c.autoTrash ? all : all.filter((it) => !trash.includes(it));
    const line = (it) =>
      `<li class="plain-row"><span class="ficon">${ico(it.action === 'trash' ? 'trash' : 'file')}</span><span class="fname" title="${esc(it.srcLabel)}">${esc(it.name)}</span><span class="dest">${
        it.action === 'trash' ? L('휴지통', 'Trash') : `→ ${esc(destText(dirOf(it.dest), t))}`
      }</span></li>`;
    const delay = state.config.settings.autoDelaySec;
    return `<div class="auto-confirm">
    <div><b>${
        items.length
          ? L(`켜면 지금 ${esc(targetName(t))}에 쌓인 ${items.length}개를 이렇게 치워요`, `Turning it on tidies the ${items.length} files already in ${esc(targetName(t))} like this`)
          : L(`지금 ${esc(targetName(t))}에는 치울 게 없어요. 새 파일이 들어오면 정리해요.`, `Nothing to tidy in ${esc(targetName(t))} right now. New files will be tidied as they arrive.`)
      }</b></div>
    ${items.length ? `<ul class="rows">${items.slice(0, 8).map(line).join('')}</ul>${items.length > 8 ? `<div class="note">${L(`외 ${items.length - 8}개`, `and ${items.length - 8} more`)}</div>` : ''}` : ''}
    ${
        t.rules.some((r) => r.enabled && r.action === 'trash') || trash.length
          ? `<label class="check-line"><input type="checkbox" id="auto-trash" ${c.autoTrash ? 'checked' : ''} /> ${L(
              `휴지통으로 보내는 규칙도 자동으로 실행${trash.length ? ` (지금 ${trash.length}개)` : ''}`,
              `Also run rules that send files to the Trash${trash.length ? ` (${trash.length} now)` : ''}`,
            )}</label>`
          : ''
      }
    <div class="muted">${L(`켜고 나서 파일이 ${delay}초쯤 그대로면 정리해요. 미리보기 없이 실행되고, 기록 탭에서 되돌릴 수 있어요.`, `Files are tidied once they have been still for about ${delay} seconds. It runs without a preview and can be undone in the History tab.`)}</div>
    <div class="card-actions">
      <button class="primary small" id="auto-confirm-on">${L('자동 정리 켜기', 'Turn on auto-tidy')}</button>
      <button class="ghost small" id="auto-confirm-cancel">${L('취소', 'Cancel')}</button>
    </div>
  </div>`;
  }

  async function askAuto(id) {
    const t = state.config.targets.find((x) => x.id === id);
    if (!t || t.auto) return;
    state.autoConfirm = { id, scan: null, autoTrash: t.autoTrash !== false };
    renderSettings();
    try {
      const scan = await tidy.scan(id, true);
      if (state.autoConfirm?.id !== id) return;
      state.autoConfirm.scan = scan;
    } catch (err) {
      state.autoConfirm = null;
      toast(err.message, 5000);
    }
    renderSettings();
  }

  async function confirmAuto() {
    const c = state.autoConfirm;
    const t = c && state.config.targets.find((x) => x.id === c.id);
    state.autoConfirm = null;
    if (!t) return renderSettings();
    t.auto = true;
    t.autoTrash = c.autoTrash;
    await saveConfig();
    renderSettings();
    toast(L(`${targetName(t)} 자동 정리를 켰어요. 짝! 하고 지켜볼게요.`, `Auto-tidy on for ${targetName(t)}. I'm watching.`));
  }

  async function register(kind) {
    const btn = kind === 'code' ? $('#reg-code') : $('#reg-desktop');
    btn.disabled = true;
    btn.innerHTML = `<span class="spinner"></span>${L('등록 중…', 'Adding…')}`;
    try {
      if (kind === 'code') {
        const results = await tidy.registerClaudeCode();
        const failed = results.filter((r) => !r.ok);
        toast(
          failed.length
            ? L(`${results.length - failed.length}곳 등록, ${failed.length}곳 실패`, `Added to ${results.length - failed.length}, ${failed.length} failed`) +
              `: ${failed[0].location} — ${failed[0].error || L('확인 실패', 'check failed')}`
            : L(`${results.length}곳에 등록했어요. 새 Claude Code 세션부터 쓸 수 있어요.`, `Added to ${results.length} places. New Claude Code sessions can use it.`),
          6000,
        );
      } else {
        const file = await tidy.registerClaudeDesktop();
        toast(L(`${file}에 추가했어요. Claude Desktop을 다시 시작해 주세요.`, `Added to ${file}. Restart Claude Desktop.`), 5000);
      }
    } catch (err) {
      toast(L(`등록 실패: ${err.message}`, `Couldn't add it: ${err.message}`), 6000);
    }
    await refreshIntegration();
    renderSettings();
  }

  root.SmashSettings = { refreshIntegration, renderUpdate, checkUpdate, renderSettings, askAuto, confirmAuto, register };
})(window);
