//! OpenCode plugin source. Its bound session emits lifecycle events to the helper.
use super::*;

pub(crate) fn opencode_plugin(helper: &Path) -> String {
    let helper = json!(helper.to_string_lossy()).to_string();
    format!(
        r"// {MARKER}
import {{ spawn }} from 'node:child_process';
import {{ randomUUID }} from 'node:crypto';
const TYPES = ['session.created','session.status','session.idle','session.error','session.deleted','permission.asked','permission.replied','permission.v2.asked','permission.v2.replied','question.asked','question.replied','question.rejected','question.v2.asked','question.v2.replied','question.v2.rejected'];
let started = false;
function bind() {{
  if (!process.env.TERMINATOR_SESSION_ID || started) return null;
  started = true;
  return {{ invocation: randomUUID(), seq: {{ n: 0 }} }};
}}
function emit(invocation, seq, event) {{
  const type = event && event.type;
  if (!TYPES.includes(type)) return;
  const payload = {{...event, terminator_invocation: invocation, terminator_sequence: ++seq.n, terminator_event: randomUUID()}};
  const child = spawn({helper}, ['event','opencode'], {{stdio:['pipe','ignore','ignore'], env:process.env}});
  child.on('error', () => {{}}); child.stdin.on('error', () => {{}});
  child.stdin.end(JSON.stringify(payload));
  const timer = setTimeout(() => child.kill(), 1800); timer.unref(); child.on('exit', () => clearTimeout(timer));
}}
async function server() {{
  const s = bind();
  if (!s) return {{}};
  return {{ event: async ({{ event }}) => emit(s.invocation, s.seq, event) }};
}}
export const Terminator = server;
export default {{
  id: 'terminator',
  server,
  async setup(ctx) {{
    const s = bind();
    if (!s) return;
    const c = new AbortController();
    void (async () => {{
      for await (const event of ctx.event.subscribe({{ signal: c.signal }})) emit(s.invocation, s.seq, event);
    }})();
    return () => c.abort();
  }},
}};
"
    )
}
