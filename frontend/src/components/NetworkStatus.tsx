import React, { useEffect, useState } from 'react';

export const NetworkStatus: React.FC = () => {
  const [isOnline, setIsOnline] = useState(navigator.onLine);

  useEffect(() => {
    const handleOnline = () => setIsOnline(true);
    const handleOffline = () => setIsOnline(false);

    window.addEventListener('online', handleOnline);
    window.addEventListener('offline', handleOffline);

    return () => {
      window.removeEventListener('online', handleOnline);
      window.removeEventListener('offline', handleOffline);
    };
  }, []);

  return (
    <div style={{ position: 'absolute', top: '1rem', right: '1rem', zIndex: 100 }}>
      {isOnline ? (
        <div className="status-badge" style={{ borderColor: 'rgba(69, 243, 255, 0.4)', background: 'rgba(69, 243, 255, 0.1)' }}>
          <span className="status-dot"></span>
          Mesh Linked
        </div>
      ) : (
        <div className="status-badge" style={{ borderColor: 'rgba(251, 45, 127, 0.4)', background: 'rgba(251, 45, 127, 0.1)', color: '#fb2d7f' }}>
          <span className="status-dot" style={{ backgroundColor: '#fb2d7f', boxShadow: '0 0 10px #fb2d7f', animation: 'none' }}></span>
          Offline (Local Island)
        </div>
      )}
    </div>
  );
};
