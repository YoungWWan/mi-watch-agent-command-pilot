import bundledApp from "../src-tauri/resources/watch-app.json";

export { bundledApp };

export interface QuickApp {
  package_name: string;
  version_code?: number;
}

export function findWatchApp(apps: QuickApp[]): QuickApp | null {
  return apps.find((app) => app.package_name === bundledApp.package) ?? null;
}

export function needsWatchAppUpgrade(app: QuickApp | null): boolean {
  return (
    app?.version_code !== undefined && app.version_code < bundledApp.versionCode
  );
}

export function watchAppVersion(app: QuickApp): string {
  // The watch protocol supplies versionCode only; do not label an old install
  // with the bundled package's versionName.
  return app.version_code === bundledApp.versionCode
    ? `v${bundledApp.versionName}`
    : app.version_code !== undefined
      ? `版本号 ${app.version_code}`
      : "版本未知";
}
