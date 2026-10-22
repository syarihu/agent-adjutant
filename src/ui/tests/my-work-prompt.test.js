const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const read = f => fs.readFileSync(path.join(__dirname, '..', f), 'utf8');

const calls = [];
let reply = null;
const ctx = vm.createContext({
  TextEncoder,
  TextDecoder,
  state: {},
  esc: x => String(x).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;'),
  baseName: p => (p || '').split('/').filter(Boolean).pop() || '',
  enc: encodeURIComponent,
  qcIcon: n => `<span class="material-symbols-outlined">${n}</span>`,
  boardApi: async (base, p, o) => {
    calls.push(['api', base, p, o?.method || 'GET', o?.body]);
    if (reply instanceof Error) throw reply;
    return reply;
  },
  note: (title, bad, why) => calls.push(['note', title, bad, why]),
  refreshWork: () => calls.push(['refreshWork']),
  refresh: () => calls.push(['refresh']),
  renderWorkTerm: () => calls.push(['renderWorkTerm']),
  todoFollow: () => calls.push(['todoFollow']),
  go: a => calls.push(['go', a]),
});

const run = (code, name) => vm.runInContext(code, ctx, { filename: name });

for (const file of ['util.js', 'sessions.js', 'my-work-seen.js', 'my-work-others.js', 'gate-evidence.js']) {
  run(read(file), file);
}
run('const workMerged = r => (r.task ? r.task.prState === "merged" : r.s?.branchPr?.state === "merged"); const prRefNumber = pr => /(\\d+)$/.exec(pr || "")?.[1] || "";', 'page.js');
run(read('my-work-clean.js'), 'my-work-clean.js');
run(read('my-work-prompt.js'), 'my-work-prompt.js');
run(read('todo.js'), 'todo.js');

const {
  promptButtonLabel,
  workPromptHtml,
  workPromptOf,
  workPromptText,
  workPromptKey,
  workPromptPress,
  todoPromptRowHtml,
  todoItems,
  todoListHtml,
} = vm.runInContext('({ promptButtonLabel, workPromptHtml, workPromptOf, workPromptText, workPromptKey, workPromptPress, todoPromptRowHtml, todoItems, todoListHtml })', ctx);

test('claude permission options map to 許可, 今後は聞かない, cut with …, and 断る', () => {
  const options = [
    { key: '1', label: 'Yes' },
    { key: '2', label: 'Yes, and always allow access to /home/user/project from this project' },
    { key: '3', label: 'Yes, and switch to auto mode · auto mode handles these prompts for you' },
    { key: '4', label: 'No' },
  ];
  assert.strictEqual(promptButtonLabel(options[0], 'permission', 0, 4, false), '許可');
  assert.strictEqual(promptButtonLabel(options[1], 'permission', 1, 4, false), '今後は聞かない');
  assert.strictEqual(promptButtonLabel(options[2], 'permission', 2, 4, true), 'Yes, and switch to auto …');
  assert.strictEqual(promptButtonLabel(options[3], 'permission', 3, 4, true), '断る');

  const p = { kind: 'permission', question: 'Do you want to proceed?', options, id: 'c1' };
  const html = workPromptHtml(p, { sessionId: 's1', boardSlug: 'b1' });
  assert.ok(html.includes('title="Yes, and switch to auto mode · auto mode handles these prompts for you"'));
  assert.ok(html.includes('btn-m3-primary'));
  assert.ok(html.includes('btn-m3-tonal'));
  // Only the first button has btn-m3-primary
  const primaryCount = (html.match(/btn-m3-primary/g) || []).length;
  assert.strictEqual(primaryCount, 1);
});

test('agy permission options map to 許可, 今後は聞かない, cut with …, and 断る', () => {
  const options = [
    { key: '1', label: 'Yes, run command' },
    { key: '2', label: "Yes, and always allow in this conversation for commands that start with 'touch'" },
    { key: '3', label: "Yes, and always allow for commands that start with 'touch' (Persist to settings.json)" },
    { key: '4', label: 'No, cancel' },
  ];
  assert.strictEqual(promptButtonLabel(options[0], 'permission', 0, 4, false), '許可');
  assert.strictEqual(promptButtonLabel(options[1], 'permission', 1, 4, false), '今後は聞かない');
  assert.strictEqual(promptButtonLabel(options[2], 'permission', 2, 4, true), 'Yes, and always allow fo…');
  assert.strictEqual(promptButtonLabel(options[3], 'permission', 3, 4, true), '断る');
});

test('choice prompt keeps labels and question text', () => {
  const options = [
    { key: '1', label: 'Green tea' },
    { key: '2', label: 'Black tea' },
  ];
  assert.strictEqual(promptButtonLabel(options[0], 'choice', 0, 2, false), 'Green tea');
  assert.strictEqual(promptButtonLabel(options[1], 'choice', 1, 2, false), 'Black tea');

  const p = { kind: 'choice', question: 'Which tea do you prefer?', options, id: 'tea1' };
  assert.strictEqual(workPromptText(p, null), 'Which tea do you prefer?');
  const html = workPromptHtml(p, { sessionId: 's1', boardSlug: 'b1' });
  assert.ok(html.includes('Which tea do you prefer?'));
  assert.ok(html.includes('Green tea'));
  assert.ok(html.includes('Black tea'));
  assert.ok(html.includes('ターミナルで聞かれています'));
  assert.ok(!html.includes('許可'));
  assert.ok(!html.includes('断る'));
});

test('permission text uses the ledger request and falls back to detail', () => {
  const p = {
    kind: 'permission',
    question: 'Do you want to proceed?',
    detail: ['Bash command', 'touch probe.txt'],
    options: [{ key: '1', label: 'Yes' }, { key: '2', label: 'No' }],
    id: 'p1',
  };

  // Uses ledger request when available
  const sessionWithReq = {
    id: 's1',
    agentSession: { request: 'Bash: npm test' },
  };
  assert.strictEqual(workPromptText(p, sessionWithReq), 'Bash: npm test を実行していい？');

  // Falls back to detail lines when request is missing
  assert.strictEqual(workPromptText(p, null), 'Bash command: touch probe.txt を実行していい？');

  // Falls back to detail lines when request is an AskUserQuestion
  const sessionAsk = {
    id: 's2',
    agentSession: { request: 'AskUserQuestion: Which one?' },
  };
  assert.strictEqual(workPromptText(p, sessionAsk), 'Bash command: touch probe.txt を実行していい？');

  // Falls back to generic text when neither is available
  const pNoDetail = { ...p, detail: [] };
  assert.strictEqual(workPromptText(pNoDetail, null), '許可を求めています');

  // The English question "Do you want to proceed?" must not be used as text for permission prompt
  assert.ok(!workPromptText(p, sessionWithReq).includes('Do you want to proceed?'));
  assert.ok(!workPromptText(p, null).includes('Do you want to proceed?'));
  assert.ok(!workPromptText(pNoDetail, null).includes('Do you want to proceed?'));
});

test('unreadable prompt shows ターミナルで答える and the 読み取れませんでした line', () => {
  const p = {
    kind: 'choice',
    question: 'Cannot read options',
    detail: [],
    options: [],
    id: 'unreadable1',
  };
  const html = workPromptHtml(p, { sessionId: 's1', boardSlug: 'b1' });
  assert.ok(html.includes('ターミナルで答える'));
  assert.ok(html.includes('画面から選択肢を読み取れませんでした'));
  assert.ok(!html.includes('data-wk-prompt-key'));
});

test('workPromptOf returns prompt only when present and ungated', () => {
  const prompt = { kind: 'permission', question: 'Allow?', options: [], id: 'p1' };

  // Present and active
  const entry = {
    board: 'b1',
    session: { id: 's1', present: true, prompt },
    gates: [],
  };
  assert.deepStrictEqual(workPromptOf(entry), prompt);

  // Gated entry returns null
  const gated = {
    board: 'b1',
    session: { id: 's1', present: true, prompt },
    gates: [{ kind: 'gate', key: 'b1/g1' }],
  };
  assert.strictEqual(workPromptOf(gated), null);

  // Not present returns null
  const absent = {
    board: 'b1',
    session: { id: 's1', present: false, prompt },
    gates: [],
  };
  assert.strictEqual(workPromptOf(absent), null);
});

test('TODO counts prompts and draws them before clean rows with escaped HTML', () => {
  const p = {
    kind: 'permission',
    question: 'Allow <script>?',
    detail: ['<b>cmd</b>', 'x & y'],
    options: [{ key: '1', label: 'Yes <ok>' }, { key: '2', label: 'No' }],
    id: 'p1',
  };

  const doc = {
    repos: [{
      nwo: 'acme/repo',
      rows: [
        {
          board: 'b1',
          session: { id: 's1', present: true, prompt: p, agentSession: { request: 'Bash: <tag>', updatedAt: 100 } },
        },
      ],
      hubSessions: [],
      turns: [],
    }],
  };

  const items = todoItems({ doc });
  assert.strictEqual(items.count, 1);
  assert.strictEqual(items.prompts.length, 1);
  // The row names the request the ledger has, as the terminal line does.
  assert.ok(items.prompts[0].text.endsWith('Bash: <tag> を実行していい？'));
  const row = todoPromptRowHtml(items.prompts[0]);
  assert.ok(row.includes('Bash: &lt;tag&gt; を実行していい？'), row);

  // Add a clean item to verify ordering in todoListHtml
  items.clean = [{
    key: 'b1/s2',
    board: 'b1',
    sessionId: 's2',
    nwo: 'acme/repo',
    title: 'Clean Task',
    text: 'Clean Task: マージ済み。片付けてセッションを閉じる？',
    files: 0,
    unpushed: 0,
    since: 200,
  }];
  items.count = 2;

  const html = todoListHtml(items);
  const promptIdx = html.indexOf('todo-prompt');
  const cleanIdx = html.indexOf('todo-clean');
  assert.ok(promptIdx >= 0 && cleanIdx >= 0);
  assert.ok(promptIdx < cleanIdx, 'prompt row must come before clean row');

  // Verify escaping
  assert.ok(!html.includes('<script>'));
  assert.ok(html.includes('&lt;tag&gt;'));
  assert.ok(html.includes('title="Yes &lt;ok&gt;"'));
});

test('workPromptPress handles 409 and other errors correctly', async () => {
  calls.length = 0;
  reply = Object.assign(new Error('conflict'), { status: 409 });
  await workPromptPress('b1', 's1', 'p1', '1');
  assert.deepStrictEqual(calls.find(c => c[0] === 'note'), ['note', '答える', true, 'この質問は、もう待っていません']);
  assert.ok(calls.some(c => c[0] === 'refreshWork'));

  calls.length = 0;
  reply = new Error('server failed');
  await workPromptPress('b1', 's1', 'p1', '1');
  assert.deepStrictEqual(calls.find(c => c[0] === 'note'), ['note', '答える', true, 'server failed']);
});
