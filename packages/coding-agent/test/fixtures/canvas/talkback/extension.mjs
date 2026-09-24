/**
 * A canvas that talks back: `session.send` from an action, and `session.on` for
 * the agent's idle signal. Used by `test/canvas-runner.test.ts`.
 */

import { createCanvas, joinSession } from "@github/copilot-sdk/extension";

const heard = [];
let session;

const canvas = createCanvas({
	id: "talkback",
	displayName: "Talkback",
	description: "Sends the agent a message when asked, and remembers the events it hears.",
	actions: [
		{
			name: "ask",
			handler: async (ctx) => ({ messageId: await session.send({ prompt: ctx.input.prompt, mode: ctx.input.mode }) }),
		},
		{ name: "heard", handler: () => heard },
	],
	open: () => ({ title: "Talkback" }),
});

session = await joinSession({ canvases: [canvas] });
session.on("session.idle", (event) => heard.push(event.type));
