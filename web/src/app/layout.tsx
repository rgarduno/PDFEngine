import type { Metadata } from 'next';
import './globals.css';

export const metadata: Metadata = {
  title: 'PDFEngine Studio | Commercial Lossless In-Place PDF Editor',
  description:
    'High-performance, lossless ISO 32000 PDF editing studio powered by safe Rust core with surgical text replacement and zero layout drift.',
};

export default function RootLayout({
  children,
}: Readonly<{
  children: React.ReactNode;
}>) {
  return (
    <html lang="en" className="h-full antialiased">
      <body className="min-h-full flex flex-col font-sans">{children}</body>
    </html>
  );
}
