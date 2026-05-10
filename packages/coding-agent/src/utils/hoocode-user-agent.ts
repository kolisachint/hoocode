export function getHooCodeUserAgent(version: string): string {
	const runtime = process.versions.bun ? `bun/${process.versions.bun}` : `node/${process.version}`;
	return `hoocode/${version} (${process.platform}; ${runtime}; ${process.arch})`;
}
