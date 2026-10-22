const fs = require('node:fs');
const vm = require('node:vm');
const ts = require('typescript');
const assert = require('node:assert/strict');
const source = fs.readFileSync('src/main.ts', 'utf8');
const render = source.slice(source.indexOf('function render('), source.indexOf('async function refresh()'));
const housings = ['codex', 'workbuddy', 'marvis', 'zcode', 'coze'].map(client => ({
  dataset: {client}, label: {},
  lights: ['waiting','completed'].map(state => ({dataset:{state}, classList:{toggle(_,active){this.active=active;}}})),
  querySelectorAll(){return this.lights;}, querySelector(){return this.label;}, setAttribute(){}
}));
const context = vm.createContext({housings,document:{body:{dataset:{}}},statusLabel:null,sessionCount:null,sessionList:null,clearSessionsButton:null,clientLabel:s=>s});
vm.runInContext(ts.transpile(render), context);
function check(sessions, expected) {
  while (expected.length < housings.length) expected.push([false,false]);
  context.render({state:'working',sessions});
  assert.deepEqual(housings.map(h=>h.lights.map(l=>l.classList.active)),expected);
}
check([{client:'codex',state:'working'},{client:'workbuddy',state:'completed'}],[[false,true],[true,false]]);
check([{client:'codex',state:'completed'},{client:'workbuddy',state:'working'}],[[true,false],[false,true]]);
check([],[[false,false],[false,false]]);
check([{client:'codex',state:'error'}],[[true,false],[false,false]]);
check([{client:'codex',state:'completed'},{client:'marvis',state:'working'}],[[true,false],[false,false],[false,true]]);
check([{client:'marvis',state:'completed'}],[[false,false],[false,false],[true,false]]);
console.log('PASS: independent client lights, reversed states, unknown, isolated error');
context.render({sessions:[{client:'marvis',state:'working'}]}, ['codex']);
assert.deepEqual(housings.map(h=>h.hidden),[false,true,true,true,true]);
assert.deepEqual(housings[0].lights.map(l=>l.classList.active),[false,false]);
context.render({sessions:[]}, []);
assert.ok(housings.every(h=>h.hidden));
context.render({sessions:[]}, ['marvis']);
assert.deepEqual(housings.map(h=>h.hidden),[true,true,false,true,true]);
context.render({sessions:[{client:'zcode',state:'working'}]}, ['zcode']);
assert.deepEqual(housings.map(h=>h.hidden),[true,true,true,false,true]);
assert.deepEqual(housings[3].lights.map(l=>l.classList.active),[false,true]);
console.log('PASS: app presence controls visibility independently of task state');
for (const [state, expected] of [['working',[false,true]],['completed',[true,false]],['idle',[false,false]]]) {
 context.render({sessions:[{client:'coze',state}]},['coze']);
 assert.deepEqual(housings.map(h=>h.hidden),[true,true,true,true,false]);
 assert.deepEqual(housings[4].lights.map(l=>l.classList.active),expected);
}
console.log('PASS: Coze red/green/unknown and app isolation');
