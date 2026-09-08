import dotenv from "dotenv";
import { fileURLToPath } from "url";
import { dirname, join } from "path";
import { google } from "googleapis";

const __dirname = dirname(fileURLToPath(import.meta.url));
dotenv.config({ path: join(__dirname, "../.env"), quiet: true });

export function getClient() {
  const client = new google.auth.OAuth2(
    process.env["CLIENT_ID"],
    process.env["CLIENT_SECRET"],
    "http://localhost"
  );
  client.setCredentials({ refresh_token: process.env["DRIVE_REFRESH_TOKEN"] });
  return client;
}
