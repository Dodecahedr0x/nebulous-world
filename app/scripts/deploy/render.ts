// Thin wrapper around Render's REST API (https://api-docs.render.com) for
// syncing the `sync: false` env vars in render.yaml and redeploying. Auth is
// read only from process.env.RENDER_API_KEY — never accepted via config file
// or CLI flag, so it can't end up committed in a deploy.config.json.

const RENDER_API_BASE = "https://api.render.com/v1";

function apiKey(): string {
  const key = process.env.RENDER_API_KEY;
  if (!key) {
    throw new Error("RENDER_API_KEY must be set in the environment to talk to Render's API");
  }
  return key;
}

async function renderRequest(path: string, init: RequestInit): Promise<void> {
  const res = await fetch(`${RENDER_API_BASE}${path}`, {
    ...init,
    headers: {
      Authorization: `Bearer ${apiKey()}`,
      "Content-Type": "application/json",
      ...init.headers,
    },
  });
  if (!res.ok) {
    throw new Error(`Render API ${init.method ?? "GET"} ${path} failed: ${res.status} ${await res.text()}`);
  }
}

/**
 * Sets a single env var on a service without touching any others — Render's
 * bulk `PUT /env-vars` replaces the *entire* set (anything omitted gets
 * deleted), so per-key updates are the only safe way to change a few values
 * without first reading back and re-sending every existing var.
 */
export async function setEnvVar(serviceId: string, key: string, value: string, dryRun: boolean): Promise<void> {
  if (dryRun) {
    console.log(`  [dry run] would set ${key} on service ${serviceId}`);
    return;
  }
  await renderRequest(`/services/${serviceId}/env-vars/${encodeURIComponent(key)}`, {
    method: "PUT",
    body: JSON.stringify({ value }),
  });
  console.log(`  set ${key} on service ${serviceId}`);
}

export async function triggerDeploy(serviceId: string, dryRun: boolean): Promise<void> {
  if (dryRun) {
    console.log(`  [dry run] would trigger a deploy of service ${serviceId}`);
    return;
  }
  await renderRequest(`/services/${serviceId}/deploys`, { method: "POST", body: JSON.stringify({}) });
  console.log(`  triggered a deploy of service ${serviceId}`);
}
