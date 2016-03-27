/**
 * Stage a native cdylib into an npm platform package (mirrors Rust `wae-dist::stage_native`).
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

/** Run `cargo build -p … --release` in `projectRoot`. */
export function cargoBuildRelease(projectRoot: string, cargoPackage: string): void {
    const cargoToml = path.join(projectRoot, 'Cargo.toml');
    if (!fs.existsSync(cargoToml)) {
        throw new Error(`project root is missing Cargo.toml: ${projectRoot}`);
    }
    const result = spawnSync('cargo', ['build', '-p', cargoPackage, '--release'], {
        cwd: projectRoot,
        stdio: 'inherit',
        shell: process.platform === 'win32',
    });
    if (result.status !== 0) {
        throw new Error(`cargo build -p ${cargoPackage} failed in ${projectRoot} (exit ${result.status ?? 'null'})`);
    }
}

/** Find a cdylib under `target/release` by package stem. */
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
    throw new Error(`no native artifact for package \`${cargoPackage}\` under ${releaseDir} (expected stem=\`${stem}\`)`);
}

/** Build and copy the cdylib into the platform package directory. */
export function stageNative(req: StageNativeRequest): string {
    if (!req.cargoPackage.trim()) throw new Error('cargo_package must not be empty');
    if (!req.destFileName.trim()) throw new Error('dest_file_name must not be empty');
    cargoBuildRelease(req.projectRoot, req.cargoPackage);
    const artifact = findCdylib(path.join(req.projectRoot, 'target', 'release'), req.cargoPackage);
    fs.mkdirSync(req.destDir, { recursive: true });
    const dest = path.join(req.destDir, req.destFileName);
    fs.copyFileSync(artifact, dest);
    return dest;
}
