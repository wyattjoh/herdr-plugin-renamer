import assert from "node:assert/strict"
import test from "node:test"
import extension from "./pi-extension.ts"

test("invokes the unique action without a plugin lookup", async () => {
  const previousPane = process.env.HERDR_PANE_ID
  process.env.HERDR_PANE_ID = "w1:p1"
  const calls: string[][] = []
  let handler: (args: string, ctx: any) => Promise<void>
  let sessionName = ""

  extension({
    registerCommand(_name: string, command: any) {
      handler = command.handler
    },
    async exec(_command: string, args: string[]) {
      calls.push(args)
      if (args[1] === "action") {
        return {
          code: 0,
          stdout: JSON.stringify({ result: { log: { log_id: "log-1" } } }),
          stderr: "",
        }
      }
      return {
        code: 0,
        stdout: JSON.stringify({
          result: {
            logs: [{ log_id: "log-1", status: "succeeded", stdout: "new-name\n" }],
          },
        }),
        stderr: "",
      }
    },
    setSessionName(name: string) {
      sessionName = name
    },
  } as any)

  try {
    await handler!("", {
      waitForIdle: async () => {},
      ui: { notify() {}, setStatus() {} },
    })
  } finally {
    if (previousPane === undefined) delete process.env.HERDR_PANE_ID
    else process.env.HERDR_PANE_ID = previousPane
  }

  assert.deepEqual(calls[0], ["plugin", "action", "invoke", "rename-current"])
  assert.equal(sessionName, "new-name")
})
