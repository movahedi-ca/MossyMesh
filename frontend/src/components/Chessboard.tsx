import React from 'react';
import './Chessboard.css';

export const Chessboard: React.FC = () => {
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
