// @ts-check
import { api, apiWritePath, headers, uiReadPath, wid, writeApi } from "./api.js";
import { selectedThreadId } from "./board.js";
import { responseError, setOut, showError, textError, unreachable } from "./feedback.js";
import { INLINE_IMAGE_TYPES, artifactFetches, artifactImages } from "./state.js";
import { artifactObjectUrls, loadMessages } from "./thread.js";



      function escapeHtml(s) {
        return s
          .replace(/&/g, "&amp;")
          .replace(/</g, "&lt;")
          .replace(/>/g, "&gt;")
          .replace(/"/g, "&quot;");
      }


      function mediaEssence(type) {
        return String(type || "").split(";")[0].trim().toLowerCase();
      }


      // The name and type come from this workspace's metadata for the sha,
      // never from the message: a message can cite any sha, and a name it
      // carried would be the poster's word, not the upload's.
      function artifactMeta(sha) {
        const key = `${wid()} ${sha}`;
        if (!artifactFetches.has(key)) {
          if (artifactFetches.size > 200) artifactFetches.clear();
          const load = api(uiReadPath(`/artifacts/${encodeURIComponent(sha)}/meta`), {
            headers: headers(),
            credentials: "include",
          }).then((res) => (res.ok ? res.json() : null));
          load.catch(() => artifactFetches.delete(key));
          artifactFetches.set(key, load);
        }
        return artifactFetches.get(key);
      }


      // Bytes go through fetch so a bearer token rides in a header, never in
      // a URL an <img> or a link could leak; a session viewer's cookie rides
      // the same request. The blob's type is set here, not taken from the
      // response, so a blob URL opened in a tab is never a document.
      async function artifactBlob(sha, type) {
        const res = await api(uiReadPath(`/artifacts/${encodeURIComponent(sha)}`), {
          headers: { ...headers(), Accept: "*/*" },
          credentials: "include",
        });
        if (!res.ok) {
          const err = new Error(await responseError(res, "Could not download the attachment"));
          err.said = true;
          throw err;
        }
        return new Blob([await res.arrayBuffer()], { type });
      }

      // The image cache is bounded in bytes as well as entries: each entry
      // holds the whole blob, and a hundred large uploads would otherwise
      // keep hundreds of MiB alive. The least recently drawn goes first.
      const ARTIFACT_IMAGE_BYTES = 32 * 1024 * 1024;
      const ARTIFACT_IMAGE_ENTRIES = 100;
      const artifactImageSizes = new Map();

      // Evict the oldest entries until both limits hold. `keep` is the image
      // being drawn now, kept even when it alone is over the budget.
      function trimImageCache(images, sizes, maxBytes, maxEntries, keep) {
        let total = 0;
        for (const n of sizes.values()) total += n;
        for (const key of images.keys()) {
          if (images.size <= maxEntries && total <= maxBytes) break;
          if (key === keep) continue;
          total -= sizes.get(key) || 0;
          sizes.delete(key);
          images.delete(key);
        }
      }

      function artifactImage(sha, type) {
        const key = `${wid()} ${sha}`;
        const cached = artifactImages.get(key);
        if (cached) {
          artifactImages.delete(key);
          artifactImages.set(key, cached);
          return cached;
        }
        const load = artifactBlob(sha, type).then((blob) => {
          if (artifactImages.get(key) === load) {
            artifactImageSizes.set(key, blob.size);
            trimImageCache(artifactImages, artifactImageSizes, ARTIFACT_IMAGE_BYTES, ARTIFACT_IMAGE_ENTRIES, key);
          }
          return blob;
        });
        load.catch(() => {
          if (artifactImages.get(key) !== load) return;
          artifactImages.delete(key);
          artifactImageSizes.delete(key);
        });
        artifactImages.set(key, load);
        trimImageCache(artifactImages, artifactImageSizes, ARTIFACT_IMAGE_BYTES, ARTIFACT_IMAGE_ENTRIES, key);
        return load;
      }


      function artifactObjectUrl(blob) {
        const url = URL.createObjectURL(blob);
        artifactObjectUrls.push(url);
        return url;
      }


      function formatBytes(n) {
        if (!Number.isFinite(n)) return "";
        if (n < 1024) return `${n} B`;
        if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
        return `${(n / (1024 * 1024)).toFixed(1)} MB`;
      }


      function artifactCard(sha) {
        const card = document.createElement("div");
        card.className = "artifact-card";
        card.dataset.sha = sha;
        const head = document.createElement("div");
        head.className = "artifact-head";
        const name = document.createElement("span");
        name.className = "artifact-name";
        name.textContent = `📎 ${sha.slice(0, 12)}…`;
        const size = document.createElement("span");
        size.className = "muted artifact-size";
        const download = document.createElement("button");
        download.type = "button";
        download.className = "artifact-download";
        download.textContent = "Download";
        head.append(name, size, download);
        card.appendChild(head);
        let filename = "";
        download.onclick = async (ev) => {
          ev.stopPropagation();
          try {
            const blob = await artifactBlob(sha, "application/octet-stream");
            const a = document.createElement("a");
            a.href = URL.createObjectURL(blob);
            a.download = filename || sha;
            a.click();
            setTimeout(() => URL.revokeObjectURL(a.href), 1000);
          } catch (e) {
            showError(e && e.said ? e.message : unreachable(e));
          }
        };
        artifactMeta(sha)
          .then(async (meta) => {
            if (!meta) {
              name.textContent = "📎 Attachment not available in this workspace";
              download.remove();
              return;
            }
            filename = meta.filename || "";
            name.textContent = `📎 ${filename || `${sha.slice(0, 12)}…`}`;
            name.title = sha;
            size.textContent = formatBytes(meta.size_bytes);
            const type = mediaEssence(meta.mime_type);
            if (!INLINE_IMAGE_TYPES.has(type)) return;
            const img = document.createElement("img");
            img.alt = filename || "Image attachment";
            img.src = artifactObjectUrl(await artifactImage(sha, type));
            card.appendChild(img);
          })
          .catch(() => {
            size.textContent = "preview unavailable";
          });
        return card;
      }


      function artifactShasFromMetadata(meta) {
        const out = [];
        if (!meta || typeof meta !== "object") return out;
        if (typeof meta.artifact_sha256 === "string") out.push(meta.artifact_sha256);
        if (typeof meta.sha256 === "string") out.push(meta.sha256);
        if (Array.isArray(meta.artifacts)) {
          meta.artifacts.forEach((item) => {
            if (typeof item === "string") out.push(item);
            else if (item && typeof item.sha256 === "string") out.push(item.sha256);
          });
        }
        return [...new Set(out)];
      }

      // Upload bytes as a content-addressed artifact. The server stores it by
      // its sha256; the file's name travels only as a display name, so a pasted
      // name cannot steer where anything is stored.
      async function uploadArtifact(blob, kind, button) {
        const params = new URLSearchParams({ kind });
        if (blob.type) params.set("mime_type", blob.type);
        if (blob.name) params.set("filename", blob.name);
        const res = await writeApi(button || null, apiWritePath(`/artifacts?${params}`), {
          method: "POST",
          headers: headers(),
          credentials: "include",
          body: blob,
        });
        const body = await res.text();
        if (!res.ok) {
          showError(textError(res.status, body, "Could not upload that file"));
          setOut(body);
          return null;
        }
        return JSON.parse(body);
      }

      async function attachToSelectedThread(artifact) {
        if (!selectedThreadId) return false;
        const pres = await writeApi(null, apiWritePath(`/threads/${selectedThreadId}/messages`), {
          method: "POST",
          headers: headers(true),
          credentials: "include",
          body: JSON.stringify({
            body: `Attached ${artifact.filename || "a file"}`,
            metadata: { artifacts: [artifact.sha256] },
          }),
        });
        if (pres.ok) {
          showError("Artifact attached in thread", "success");
          await loadMessages();
        }
        return pres.ok;
      }

export { artifactBlob, artifactCard, artifactImage, artifactMeta, artifactObjectUrl, artifactShasFromMetadata, attachToSelectedThread, escapeHtml, formatBytes, mediaEssence, trimImageCache, uploadArtifact };
