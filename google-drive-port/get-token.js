import { google } from "googleapis";
import http from "http";
import { URL } from "url";
import dotenv from "dotenv";
dotenv.config({ quiet: true });

const oauth2Client = new google.auth.OAuth2(
  process.env.CLIENT_ID,
  process.env.CLIENT_SECRET,
  "http://127.0.0.1:4242/callback"
);

const authUrl = oauth2Client.generateAuthUrl({
  access_type: "offline",
  scope: ["https://www.googleapis.com/auth/drive.file"],
  prompt: "consent",
});

import { execSync } from "child_process";
execSync(`open "${authUrl}"`);

const server = http.createServer(async (req, res) => {
  const code = new URL(req.url, "http://127.0.0.1:4242").searchParams.get("code");
  if (!code) { res.end("No code"); return; }

  res.end("Done! Check your terminal for the refresh token.");
  server.close();

  const { tokens } = await oauth2Client.getToken(code);
  console.log("\nRefresh token (save to .env as DRIVE_REFRESH_TOKEN):");
  console.log(tokens.refresh_token);
  process.exit(0);
});

server.listen(4242, () => console.log("Waiting on http://127.0.0.1:4242/callback ..."));
