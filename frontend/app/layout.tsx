import type { Metadata } from "next";

export const metadata: Metadata = {
  title: "AnchorCore",
  description: "Connect your knowledge to any AI model",
};

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en">
      <body style={{ margin: 0, fontFamily: "system-ui, sans-serif", background: "#0f1115", color: "#e6e8eb" }}>
        {children}
      </body>
    </html>
  );
}
