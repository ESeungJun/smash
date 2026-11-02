#!/usr/bin/env node
'use strict';
// Smash! MCP 서버 (stdio, 줄 단위 JSON-RPC).
// 앱과 같은 설정·기록 파일을 쓰므로 Claude에서 정리한 내역도 앱의 '기록'에 남는다.
const path = require('path');
const readline = require('readline');
const { loadConfig, saveConfig, contractHome, newId } = require('../core/config');
const { scanTarget, planFromMoves, applyPlan, undoLast, loadHistory, folderTree, findTarget } = require('../core/organizer');
const { setLang, resolveLang } = require('../core/lang');

// 앱에서 고른 언어로 오류·결과 문구를 돌려준다.
setLang(resolveLang(loadConfig().settings.language));

// 패키징하면 이 파일은 app.asar.unpacked에 있어 package.json을 못 읽을 수 있다.
function appVersion() {
  const dev = path.join(__dirname, '..', '..', 'package.json');
  for (const file of [dev, dev.replace('app.asar.unpacked', 'app.asar')]) {
    try {
      return JSON.parse(require('fs').readFileSync(file, 'utf8')).version;
    } catch {
      /* 다음 후보 */
    }
  }
  return '0.0.0';
}
const SERVER_INFO = { name: 'smash', version: appVersion() };
const PROTOCOL_VERSIONS = ['2025-06-18', '2025-03-26', '2024-11-05'];
// scan_folder가 만든 계획을 apply_plan에서 id로 받을 수 있게 잠시 들고 있는다.
const lastPlan = new Map();

const view = (item) => ({
  id: item.id,
  file: contractHome(item.src),
  action: item.action,
  dest: item.dest ? contractHome(item.dest) : null,
  note: item.note === 'duplicate' ? `같은 내용 파일이 이미 있음: ${contractHome(item.duplicateOf)}` : item.note === 'renamed' ? '같은 이름이 있어 이름 변경' : null,
  reason: item.reason,
});

const TOOLS = [
  {
    name: 'list_targets',
    description: '정리 대상으로 등록된 폴더와 각 폴더의 규칙을 보여 준다.',
    inputSchema: { type: 'object', properties: {} },
    run() {
      const cfg = loadConfig();
      return cfg.targets.map((t) => ({
        id: t.id,
        path: t.path,
        rules: t.rules.map((r) => ({
          name: r.name,
          enabled: r.enabled,
          when: r.match,
          action: r.action,
          dest: r.dest || null,
        })),
      }));
    },
  },
  {
    name: 'scan_folder',
    description:
      '정리 대상 폴더를 스캔해 규칙에 따른 정리 계획(plan)과 규칙에 안 걸린 파일(unmatched), 옮길 수 있는 폴더 목록(folders)을 돌려준다. 파일은 아직 움직이지 않는다. unmatched 파일은 이름을 보고 folders 중 알맞은 곳을 골라 apply_plan의 moves로 넘기면 된다.',
    inputSchema: {
      type: 'object',
      properties: { target: { type: 'string', description: '폴더 id(예: downloads) 또는 경로(예: ~/Downloads). 생략하면 첫 번째 폴더.' } },
    },
    run({ target }) {
      const cfg = loadConfig();
      const res = scanTarget(cfg, target);
      lastPlan.clear();
      for (const item of res.items) lastPlan.set(item.id, item);
      return {
        target: contractHome(res.root),
        plan: res.items.map(view),
        unmatched: res.unmatched.map((f) => ({ file: contractHome(f.path), sizeKB: Math.round(f.size / 1024), modified: new Date(f.mtimeMs).toISOString().slice(0, 10) })),
        folders: folderTree(cfg).map(contractHome),
      };
    },
  },
  {
    name: 'apply_plan',
    description:
      '정리를 실제로 실행한다. 실행 전에 무엇을 어디로 옮기는지 사용자에게 보여 주고 동의를 받을 것. item_ids는 scan_folder가 준 계획 항목, moves는 직접 정한 이동이다. 삭제는 없고 휴지통으로만 보낸다. 실행 내역은 undo_last로 되돌릴 수 있다.',
    inputSchema: {
      type: 'object',
      properties: {
        item_ids: { type: 'array', items: { type: 'string' }, description: 'scan_folder 결과 plan의 id 목록' },
        moves: {
          type: 'array',
          items: {
            type: 'object',
            properties: {
              src: { type: 'string', description: '옮길 파일 경로' },
              dest_folder: { type: 'string', description: '목적지 폴더 경로(절대 경로, ~ 경로, 또는 파일이 있는 폴더 기준 상대 경로)' },
              action: { type: 'string', enum: ['move', 'trash'] },
              reason: { type: 'string' },
            },
            required: ['src'],
          },
        },
      },
    },
    run({ item_ids = [], moves = [] }) {
      const cfg = loadConfig();
      const items = [];
      const rejected = [];
      for (const id of item_ids) {
        const item = lastPlan.get(id);
        if (item) items.push(item);
        else rejected.push({ src: id, reason: '계획에 없는 id입니다. scan_folder를 다시 실행하세요.' });
      }
      const fromMoves = planFromMoves(
        cfg,
        moves.map((m) => ({ src: m.src, destFolder: m.dest_folder, action: m.action || 'move', reason: m.reason ? `Claude: ${m.reason}` : 'Claude' })),
      );
      items.push(...fromMoves.items);
      rejected.push(...fromMoves.rejected);
      const { results, run } = applyPlan(cfg, items, 'claude-mcp');
      for (const id of item_ids) lastPlan.delete(id);
      return { runId: run?.id || null, results, rejected };
    },
  },
  {
    name: 'add_rule',
    description: '정리 규칙을 추가한다. 확장자·이름 포함 문자열·오래된 날짜 중 하나 이상의 조건이 필요하다. 앞에 있는 규칙이 먼저 적용된다.',
    inputSchema: {
      type: 'object',
      properties: {
        target: { type: 'string', description: '폴더 id 또는 경로' },
        name: { type: 'string' },
        extensions: { type: 'array', items: { type: 'string' }, description: '예: ["pdf","hwp"]' },
        name_includes: { type: 'array', items: { type: 'string' }, description: '파일 이름에 들어 있으면 매칭(대소문자 무시)' },
        older_than_days: { type: 'number' },
        action: { type: 'string', enum: ['move', 'trash'] },
        dest: { type: 'string', description: 'move일 때 목적지. 상대 경로면 대상 폴더 기준.' },
        position: { type: 'string', enum: ['first', 'last'], description: '기본 first' },
      },
      required: ['target', 'name', 'action'],
    },
    run(args) {
      const cfg = loadConfig();
      const target = findTarget(cfg, args.target);
      if (!target) throw new Error(`설정에 없는 폴더입니다: ${args.target}`);
      if (args.action === 'move' && !args.dest) throw new Error('move 규칙에는 dest가 필요합니다');
      const rule = {
        id: newId('r'),
        name: args.name,
        enabled: true,
        match: { extensions: args.extensions || [], nameIncludes: args.name_includes || [], olderThanDays: args.older_than_days || 0 },
        action: args.action,
        dest: args.dest || '',
      };
      if (!rule.match.extensions.length && !rule.match.nameIncludes.length && !rule.match.olderThanDays) throw new Error('조건이 하나 이상 필요합니다');
      if (args.position === 'last') target.rules.push(rule);
      else target.rules.unshift(rule);
      saveConfig(cfg);
      return { added: rule.name, target: target.path };
    },
  },
  {
    name: 'undo_last',
    description: '가장 최근 정리 실행을 되돌린다(옮긴 파일을 원래 자리로). 휴지통으로 보낸 파일은 사용자가 휴지통에서 직접 복원해야 한다.',
    inputSchema: { type: 'object', properties: {} },
    run: () => undoLast(),
  },
  {
    name: 'history',
    description: '최근 정리 실행 기록을 보여 준다.',
    inputSchema: { type: 'object', properties: { limit: { type: 'number' } } },
    run({ limit = 5 }) {
      return loadHistory()
        .slice(-limit)
        .reverse()
        .map((r) => ({
          id: r.id,
          at: r.at,
          source: r.source,
          undone: r.undone,
          ops: r.ops.map((o) => (o.type === 'move' ? `${contractHome(o.from)} → ${contractHome(o.to)}` : `휴지통: ${contractHome(o.from)}`)),
        }));
    },
  },
];

function send(msg) {
  process.stdout.write(`${JSON.stringify(msg)}\n`);
}

function handle(msg) {
  const { id, method, params = {} } = msg;
  const reply = (result) => id !== undefined && send({ jsonrpc: '2.0', id, result });
  const fail = (code, message) => id !== undefined && send({ jsonrpc: '2.0', id, error: { code, message } });
  switch (method) {
    case 'initialize': {
      const asked = params.protocolVersion;
      return reply({
        protocolVersion: PROTOCOL_VERSIONS.includes(asked) ? asked : PROTOCOL_VERSIONS[0],
        capabilities: { tools: {} },
        serverInfo: SERVER_INFO,
        instructions:
          'Smash!: 사용자의 폴더(다운로드·문서 등)를 규칙대로 정리하는 도구. scan_folder로 계획을 만들고, 사용자에게 보여 준 뒤 apply_plan으로 실행한다. 삭제 없이 휴지통으로만 보내며 undo_last로 되돌릴 수 있다.',
      });
    }
    case 'ping':
      return reply({});
    case 'tools/list':
      return reply({ tools: TOOLS.map(({ name, description, inputSchema }) => ({ name, description, inputSchema })) });
    case 'tools/call': {
      const tool = TOOLS.find((t) => t.name === params.name);
      if (!tool) return fail(-32602, `없는 도구: ${params.name}`);
      try {
        const out = tool.run(params.arguments || {});
        return reply({ content: [{ type: 'text', text: JSON.stringify(out, null, 2) }] });
      } catch (err) {
        return reply({ content: [{ type: 'text', text: `오류: ${err.message}` }], isError: true });
      }
    }
    default:
      if (method?.startsWith('notifications/')) return undefined;
      return fail(-32601, `지원하지 않는 메서드: ${method}`);
  }
}

const rl = readline.createInterface({ input: process.stdin });
rl.on('line', (line) => {
  if (!line.trim()) return;
  let msg;
  try {
    msg = JSON.parse(line);
  } catch {
    return send({ jsonrpc: '2.0', id: null, error: { code: -32700, message: 'Parse error' } });
  }
  handle(msg);
});
rl.on('close', () => process.exit(0));
