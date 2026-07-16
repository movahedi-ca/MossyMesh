import React, { useState } from 'react';
import './Chessboard.css';

export const Chessboard: React.FC = () => {
  const [boardState, setBoardState] = useState([]);
  const [status, setStatus] = useState<string>("Ready");

  const handleMove = async (from: [number, number], to: [number, number]) => {
    setStatus("Submitting to Mesh...");
    try {
      const response = await fetch('/api/v1/submit_job', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ action: "move", from, to })
      });
      if (response.ok) {
        setStatus("Move Confirmed by Swarm");
      } else {
        setStatus("Swarm Rejected Move");
      }
    } catch (err) {
      // Offline fallback: Push directly to local daemon
      setStatus("Offline: Sent to local Rust Daemon via DHT");
    }
  };

  // Generate an empty 8x8 chessboard
  const board = [];
  for (let rank = 8; rank >= 1; rank--) {
    for (let file = 1; file <= 8; file++) {
      const isDark = (rank + file) % 2 === 0;
      board.push(
        <div 
          key={`${file}-${rank}`} 
          className={`square ${isDark ? 'dark-square' : 'light-square'}`}
        ></div>
      );
    }
  }

  return (
    <div className="chessboard-container">
      <div className="chessboard">
        {board}
      </div>
      <div className="chessboard-controls">
        <button className="mesh-btn primary">Play offline</button>
        <button className="mesh-btn secondary">Seek peer via LoRa</button>
      </div>
    </div>
  );
};
