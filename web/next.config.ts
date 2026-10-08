import type { NextConfig } from "next";

/**
 * Origins the studio may call.
 *
 * The PDF API is a different port from this Next process. `connect-src`
 * therefore names that origin and its WebSocket scheme. A custom origin is
 * read from NEXT_PUBLIC_API_URL when this process starts. The local defaults
 * stay so the usual backend on port 8000 still connects.
 *
 * Next inlines scripts and Tailwind writes style attributes, so script-src
 * and style-src keep 'unsafe-inline'. The dev compiler also needs
 * 'unsafe-eval'. Images and fonts are fetched with the bearer token and then
 * shown from object URLs, so img-src allows blob: and the API itself is not
 * an image origin.
 */
function studioConnectSources(): string {
  const configured = process.env.NEXT_PUBLIC_API_URL || "http://localhost:8000";
  const sources = new Set<string>([
    "'self'",
    "http://127.0.0.1:8000",
    "http://localhost:8000",
    "ws://127.0.0.1:8000",
    "ws://localhost:8000",
  ]);
  try {
    const url = new URL(configured);
    sources.add(url.origin);
    const socketScheme = url.protocol === "https:" ? "wss:" : "ws:";
    sources.add(`${socketScheme}//${url.host}`);
  } catch {
    // An unparseable override does not drop the local API origins above.
  }
  return Array.from(sources).join(" ");
}

const contentSecurityPolicy = [
  "default-src 'self'",
  "script-src 'self' 'unsafe-inline' 'unsafe-eval'",
  "style-src 'self' 'unsafe-inline'",
  "img-src 'self' data: blob:",
  "font-src 'self' data:",
  `connect-src ${studioConnectSources()}`,
  "frame-ancestors 'none'",
  "base-uri 'self'",
  "form-action 'self'",
  "object-src 'none'",
].join("; ");

const securityHeaders = [
  { key: "Content-Security-Policy", value: contentSecurityPolicy },
  { key: "X-Content-Type-Options", value: "nosniff" },
  { key: "X-Frame-Options", value: "DENY" },
  { key: "Referrer-Policy", value: "strict-origin-when-cross-origin" },
  {
    key: "Permissions-Policy",
    value: "camera=(), microphone=(), geolocation=(), payment=()",
  },
];

const nextConfig: NextConfig = {
  webpack: (config, { isServer }) => {
    if (!isServer) {
      config.resolve = config.resolve || {};
      config.resolve.fallback = {
        ...config.resolve.fallback,
        canvas: false,
        fs: false,
      };
    }
    return config;
  },
  async headers() {
    return [
      {
        source: "/:path*",
        headers: securityHeaders,
      },
    ];
  },
};

export default nextConfig;
