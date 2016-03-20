/**
 * 在 `node --test --experimental-strip-types` 前注册，使源码内 `.js` 导入落到同名 `.ts`。
 */
import { register } from 'node:module';

register('./resolve-ts-from-js-hook.mjs', import.meta.url);
