export interface LatestHooCodeRelease {
    version: string;
    packageName?: string;
}
export declare function comparePackageVersions(leftVersion: string, rightVersion: string): number | undefined;
export declare function isNewerPackageVersion(candidateVersion: string, currentVersion: string): boolean;
export declare function getLatestHooCodeRelease(currentVersion: string, options?: {
    timeoutMs?: number;
}): Promise<LatestHooCodeRelease | undefined>;
export declare function getLatestHooCodeVersion(currentVersion: string, options?: {
    timeoutMs?: number;
}): Promise<string | undefined>;
export declare function checkForNewHooCodeVersion(currentVersion: string): Promise<string | undefined>;
//# sourceMappingURL=version-check.d.ts.map