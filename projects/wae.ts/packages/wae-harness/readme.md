# @wae/wae-harness

Automation harness for WAE WebView apps.

- In-process driver over the **shared** `@wae/wae-<platform>-<arch>` addon: `Harness.open`, `eval`, `waitFor`, `screenshot`, `takeIpc`, `close`
- External driver CLI (`wae-harness`) that starts an app with a loopback control channel and replays a JSON operations file
- Windows only for script evaluation and screenshots; other platforms fail with structured unsupported errors
- No separate native build and no second `.node`: the harness uses the same platform packages as `@wae/wae`

```ts
import { Harness } from '@wae/wae-harness';

const harness = Harness.open({ url: 'data:text/html,<h1>hi</h1>', hidden: true });
harness.waitFor("document.readyState === 'complete'", 5000);
console.log(harness.eval('document.title'));
harness.close();
```

```
wae-harness run --app ./target/debug/my-app --script ops.json
```
