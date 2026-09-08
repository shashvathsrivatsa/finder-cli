import { createReadStream } from "fs";
import { extname, basename } from "path";
import { google } from "googleapis";
import { getClient } from "./auth.js";
import { getMimeInfo, getFileUrl } from "./mimeTypes.js";

async function upload(filePath: string): Promise<void> {
  const ext = extname(filePath).toLowerCase();
  const { sourceMime, targetMime, appType } = getMimeInfo(ext);

  const drive = google.drive({ version: "v3", auth: getClient() });

  const res = await drive.files.create({
    requestBody: {
      name: basename(filePath, ext),
      mimeType: targetMime,
    },
    media: {
      mimeType: sourceMime,
      body: createReadStream(filePath),
    },
    fields: "id",
  });

  const fileId = res.data.id;
  if (!fileId) throw new Error("Upload succeeded but no file ID returned");

  const url = getFileUrl(fileId, appType);
  process.stdout.write(`${url}\n`);
}

const filePath = process.argv[2];
if (!filePath) {
  process.stderr.write("Usage: node dist/main.js <file>\n");
  process.exit(1);
}

upload(filePath).catch((e) => {
  process.stderr.write(`Error: ${e.message}\n`);
  process.exit(1);
});
