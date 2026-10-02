'use client';

import React, { useEffect, useState } from 'react';
import { fetchAuthorizedBuffer } from '@/lib/api';

interface AuthorizedImageProps {
  url: string;
  alt: string;
  className?: string;
}

/**
 * Loads an API image with the studio bearer token.
 * An <img src> request cannot attach Authorization, so the bytes are fetched
 * and shown through a short-lived object URL.
 */
export function AuthorizedImage({ url, alt, className }: AuthorizedImageProps) {
  const [objectUrl, setObjectUrl] = useState<string | null>(null);

  useEffect(() => {
    let revoked = false;
    let current: string | null = null;

    fetchAuthorizedBuffer(url)
      .then((buffer) => {
        if (revoked) return;
        current = URL.createObjectURL(new Blob([buffer]));
        setObjectUrl(current);
      })
      .catch(() => {
        if (!revoked) setObjectUrl(null);
      });

    return () => {
      revoked = true;
      if (current) URL.revokeObjectURL(current);
    };
  }, [url]);

  if (!objectUrl) {
    return <div className={className} role="img" aria-label={alt} />;
  }

  return <img src={objectUrl} alt={alt} className={className} />;
}
