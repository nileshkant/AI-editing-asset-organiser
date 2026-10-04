import { useState } from 'react';
import toolContract from '../../../docs/MCP_TOOLS.md?raw';
import accessGuide from '../../../docs/MCP_ACCESS.md?raw';

// Bundle the maintained contract so installed/offline readers see the same details
// as repository readers, rather than a second, drifting list of tools.
function Inline({ text }: { text: string }) {
  return <>{text.split(/(`[^`]+`|\*\*[^*]+\*\*|\[[^\]]+\]\([^)]+\))/g).map((part, i) => {
    if (part.startsWith('`')) return <code key={i}>{part.slice(1, -1)}</code>;
    if (part.startsWith('**')) return <strong key={i}>{part.slice(2, -2)}</strong>;
    const link = part.match(/^\[([^\]]+)\]\(([^)]+)\)$/);
    if (link) return <span key={i}>{link[1]} ({link[2]})</span>;
    return part;
  })}</>;
}
export function ReferenceDocument({ text }: { text: string }) {
  return <div className="mcp-reference">{text.replace(/\r\n?/g, '\n').split(/\n\n+/).map((block, index) => {
    if (block.startsWith('# ')) return <h3 key={index}>{block.slice(2)}</h3>;
    if (block.startsWith('## ')) return <h4 key={index}>{block.slice(3)}</h4>;
    if (block.startsWith('- ')) return <ul key={index}>{block.split('\n').map((line, i) => <li key={i}><Inline text={line.replace(/^- /, '')} /></li>)}</ul>;
    return <p key={index}><Inline text={block} /></p>;
  })}</div>;
}
export function McpGuide() {
  const [reference, setReference] = useState<'tools' | 'access'>('tools');
  return <article aria-label="MCP guide">
    <div className="settings-card">
      <h2>MCP guide</h2>
      <p>Connect an AI assistant to your CreativeShelf audio catalog. MCP provides tools; the client supplies the AI model. You can search, add annotations, create clips and export to a destination you approve.</p>
      <h3>Set up once</h3>
      <ol>
        <li>Use Add folder or Add files to import audio in CreativeShelf. Originals remain in your source folders; wait until the import finishes.</li>
        <li>Open Settings → Agent access and Start MCP. Leave port 0 for first setup or enter a free port. The endpoint ends in <code>/mcp</code>.</li>
        <li>Enter a separate name for each client, choose allowed sources and permissions, then Pair client. Read is separate from Edit, Export and Reveal local paths. No selected sources means empty results.</li>
        <li>Copy the displayed configuration into a client that supports HTTP MCP and Authorization headers. The bearer credential is shown once; save it privately in that client.</li>
        <li>For a command-based client, Show installed bridge gives the exact executable and discovery arguments. Set <code>SOUNDSHELF_MCP_TOKEN</code> in its private environment. Client configuration formats differ; use the client's MCP settings.</li>
      </ol>
      <p>MCP starts with CreativeShelf after it is enabled, using the remembered port and existing tokens. CreativeShelf must remain open. Stop MCP disables automatic startup and keeps pairings; Quit only closes access until next launch. Rotate token explicitly replaces one credential; Revoke removes a client. Revoke and pair again to change its permissions.</p>
      <p>Upgrading from alpha.5 requires pairing once more because older credentials were memory-only. On this version, subsequent restarts preserve them.</p>
      <h3>Search and tag audio with an AI client</h3>
      <ol>
        <li>Ask the client to call <code>sources.list</code>, then <code>sounds.search</code> for its allowed sources. Use tags, text, favorites and duration filters. Fetch a result with <code>sounds.get</code>, retaining its id and content version.</li>
        <li>For sound recognition, use an audio-capable model that can actually listen to the recording. CreativeShelf's MCP does not stream audio or run a recognition model. Path permission plus <code>sounds.resolve</code> can reveal one scoped local path; the client must separately support local audio input. Obtain consent before a client sends audio to a cloud provider.</li>
        <li>Review specific, supported event labels such as rain, thunder, footsteps, engine or hiss. Do not infer a sound solely from a filename, duration or pace. Ask the client to preserve existing annotations and call <code>sounds.annotate</code> with the current id/version and merged tags. Edit permission is required.</li>
        <li>Search those tags in Library or through <code>sounds.search</code>. Inspect the result and correct mistaken AI labels.</li>
      </ol>
      <p><strong>Current import limitation:</strong> MCP cannot import files, add sources or trigger rescans. Import in the app first. Automatic audio recognition on import is planned in SS-016/SS-017; approved MCP import and an AI tagging workflow are tracked in SS-039. These are upcoming features, not enabled capabilities.</p>
      <h3>Create and export a clip</h3>
      <p>Use <code>clips.create</code> with source-frame boundaries and the sound's current version. Read and Edit permissions are needed. For export, enable Export for that client and approve an exact WAV/FLAC destination in Agent access. Pass that destination ID to <code>clips.export</code>, then poll <code>jobs.get</code>. Approvals expire after ten minutes and permit one use; originals and existing outputs are never replaced.</p>
      <h3>API endpoint and troubleshooting</h3>
      <p>The single HTTP MCP endpoint is <code>http://127.0.0.1:PORT/mcp</code>. Tool names below are MCP tools, not individual REST paths. An MCP client performs initialization, discovery and tool calls. Send the credential in <code>Authorization: Bearer TOKEN</code>, never a URL query.</p>
      <ul>
        <li>Stopped after reopening: check the startup error in Agent access. Close the service occupying the remembered port or choose another explicit port and update the client URL.</li>
        <li>Unauthorized: verify the client's token and header. If you lost the token or rotated it, update that client's configuration with the new one.</li>
        <li>Empty results or permission denied: check selected sources, Read/Edit/Export/Paths and source availability. Re-pair to change grants.</li>
        <li>Changed or missing audio: refresh its current version, check the source location and rescan in the app. Do not retry a stale clip recipe blindly.</li>
        <li>Export fails: check its approved destination, collision, expiry and job status. Reuse an idempotency key only for exactly the same operation. Session job/retry records do not survive shutdown.</li>
      </ul>
    </div>
    <div className="settings-card">
      <h3>Complete bundled reference</h3>
      <label className="settings-control">Reference document <select value={reference} onChange={event => setReference(event.target.value as 'tools' | 'access')}>
        <option value="tools">Every tool, argument and error</option><option value="access">Transport, credentials and bridge</option>
      </select></label>
      <ReferenceDocument text={reference === 'tools' ? toolContract : accessGuide} />
    </div>
  </article>;
}
