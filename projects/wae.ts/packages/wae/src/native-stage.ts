/**
 * 原生 cdylib → npm 平台包暂存（对齐 Rust `wae-dist::stage_native`）。
 */
import fs from 'node:fs';
import path from 'node:path';
import { spawnSync } from 'node:child_process';

export type StageNativeRequest = {
    projectRoot: string;
    cargoPackage: string;
    destDir: string;
    destFileName: string;
};

/** 在 projectRoot 执行 `cargo build -p … --release`。 */
export function cargoBuildRelease(projectRoot: string, cargoPackage: string): void {
    const cargoToml = path.join(projectRoot, 'Cargo.toml');
    if (!fs.existsSync(cargoToml)) {
        throw new Error(`项目根缺少 Cargo.toml：${projectRoot}`);
    }
    const result = spawnSync('cargo', ['build', '-p', cargoPackage, '--release'], {
        cwd: projectRoot,
        stdio: 'inherit',
        shell: process.platform === 'win32',
    });
    if (result.status !== 0) {
        throw new Error(`在 ${projectRoot} 构建 -p ${cargoPackage} 失败（exit ${result.status ?? 'null'}）`);
    }
}

/** 在 `target/release` 下按包名 stem 查找 cdylib。 */
export function findCdylib(releaseDir: string, cargoPackage: string): string {
    const stem = cargoPackage.replaceAll('-', '_');
    const candidates = [
        path.join(releaseDir, `${stem}.dll`),
        path.join(releaseDir, `${stem}.node`),
        path.join(releaseDir, `lib${stem}.so`),
        path.join(releaseDir, `lib${stem}.dylib`),
    ];
    for (const c of candidates) {
        if (fs.existsSync(c)) return c;
    }
    throw new Error(`在 ${releaseDir} 下未找到包 \`${cargoPackage}\` 的原生产物（期望 stem=\`${stem}\`）`);
}

/** 构建并把 cdylib 拷到平台包目录。 */
export function stageNative(req: StageNativeRequest): string {
    if (!req.cargoPackage.trim()) throw new Error('cargo_package 不能为空');
    if (!req.destFileName.trim()) throw new Error('dest_file_name 不能为空');
    cargoBuildRelease(req.projectRoot, req.cargoPackage);
    const artifact = findCdylib(path.join(req.projectRoot, 'target', 'release'), req.cargoPackage);
    fs.mkdirSync(req.destDir, { recursive: true });
    const dest = path.join(req.destDir, req.destFileName);
    fs.copyFileSync(artifact, dest);
    return dest;
}
