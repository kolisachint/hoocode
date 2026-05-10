export function getHooCodeUserAgent(version) {
    const runtime = process.versions.bun ? `bun/${process.versions.bun}` : `node/${process.version}`;
    return `hoocode/${version} (${process.platform}; ${runtime}; ${process.arch})`;
}
//# sourceMappingURL=hoocode-user-agent.js.map