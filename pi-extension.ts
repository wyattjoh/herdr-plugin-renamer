import type { ExtensionAPI } from "@earendil-works/pi-coding-agent"

const PLUGIN_ID = "herdr-plugin-renamer"
const ACTION_ID = "rename-current"

function record(value: unknown): Record<string, unknown> | undefined {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : undefined
}

function parseJson(stdout: string): Record<string, unknown> | undefined {
  try {
    return record(JSON.parse(stdout))
  } catch {
    return undefined
  }
}

function invocationLogId(stdout: string): string | undefined {
  const result = record(parseJson(stdout)?.result)
  const log = record(result?.log)
  return typeof log?.log_id === "string" ? log.log_id : undefined
}

function commandLog(stdout: string, logId: string): Record<string, unknown> | undefined {
  const result = record(parseJson(stdout)?.result)
  const logs = Array.isArray(result?.logs) ? result.logs : []
  return logs.map(record).find((log) => log?.log_id === logId)
}

async function waitForName(pi: ExtensionAPI, herdr: string, logId: string): Promise<string> {
  for (let attempt = 0; attempt < 120; attempt += 1) {
    const result = await pi.exec(
      herdr,
      ["plugin", "log", "list", "--plugin", PLUGIN_ID, "--limit", "20"],
      { timeout: 5000 },
    )
    if (result.code !== 0) throw new Error(result.stderr.trim() || "Could not read plugin log")

    const log = commandLog(result.stdout, logId)
    const status = log?.status
    if (status === "succeeded") {
      const name =
        typeof log.stdout === "string"
          ? log.stdout.trim().split(/\r?\n/).at(-1)?.trim() || ""
          : ""
      if (!name) throw new Error("Renamer returned no name")
      return name
    }
    if (status === "failed") {
      const error = typeof log.stderr === "string" ? log.stderr.trim() : ""
      throw new Error(error || "Renamer failed")
    }
    await new Promise((resolve) => setTimeout(resolve, 500))
  }
  throw new Error("Renamer timed out")
}

export default function (pi: ExtensionAPI): void {
  pi.registerCommand("rename", {
    description: "Generate a new Pi and Herdr worktree name",
    handler: async (args, ctx) => {
      if (args.trim()) {
        ctx.ui.notify("Usage: /rename", "warning")
        return
      }
      if (!process.env.HERDR_PANE_ID) {
        ctx.ui.notify("/rename is only available inside Herdr", "warning")
        return
      }

      await ctx.waitForIdle()
      const herdr = process.env.HERDR_BIN_PATH || "herdr"
      ctx.ui.setStatus("herdr-rename", "renaming…")
      try {
        const invoked = await pi.exec(
          herdr,
          ["plugin", "action", "invoke", ACTION_ID],
          { timeout: 5000 },
        )
        if (invoked.code !== 0) {
          throw new Error(invoked.stderr.trim() || "Could not start renamer")
        }
        const logId = invocationLogId(invoked.stdout)
        if (!logId) throw new Error("Renamer returned no command id")

        const name = await waitForName(pi, herdr, logId)
        pi.setSessionName(name)
        ctx.ui.notify(`Session and Herdr pane renamed: ${name}`, "info")
      } catch (error) {
        ctx.ui.notify(error instanceof Error ? error.message : String(error), "error")
      } finally {
        ctx.ui.setStatus("herdr-rename", undefined)
      }
    },
  })
}
