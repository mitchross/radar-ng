// Radar-NG — illustrated weather icons
// "Pixel-y but polished" — rounded-square character vibe like Carrot,
// but drawn with SVG primitives so they scale cleanly.
//
// Each icon is an SVG with a defined viewBox of 64x64.
// Usage: <WeatherIcon kind="rain" size={40} />

function WeatherIcon({ kind, size = 40, time = 'day' }) {
  const ICON = ICONS[kind] || ICONS.cloudy;
  return (
    <svg width={size} height={size} viewBox="0 0 64 64" style={{ display: 'block' }}>
      {ICON(time)}
    </svg>
  );
}

// Shared pieces
const SunBody = ({ cx = 32, cy = 32, r = 11, color = '#FFC14D' }) => (
  <>
    <circle cx={cx} cy={cy} r={r + 6} fill={color} opacity="0.18" />
    <circle cx={cx} cy={cy} r={r + 3} fill={color} opacity="0.28" />
    <circle cx={cx} cy={cy} r={r} fill={color} />
    <circle cx={cx - 3} cy={cy - 3} r={r * 0.35} fill="#FFE7A8" opacity="0.9" />
  </>
);

const MoonBody = ({ cx = 32, cy = 30, r = 11 }) => (
  <>
    <circle cx={cx} cy={cy} r={r + 4} fill="#C7B8FF" opacity="0.15" />
    <circle cx={cx} cy={cy} r={r} fill="#E8DFFF" />
    <circle cx={cx + 4} cy={cy - 2} r={r * 0.95} fill="#241858" />
    <circle cx={cx - 3} cy={cy + 3} r="1.4" fill="#B5A6E8" opacity="0.6" />
    <circle cx={cx - 5} cy={cy - 2} r="0.9" fill="#B5A6E8" opacity="0.5" />
  </>
);

const CloudBody = ({ cx = 32, cy = 36, scale = 1, color = '#E8ECF5', shade = '#9AA4BE' }) => {
  const s = scale;
  return (
    <g transform={`translate(${cx - 22 * s}, ${cy - 12 * s}) scale(${s})`}>
      {/* shadow */}
      <ellipse cx="22" cy="22" rx="22" ry="4" fill="#000" opacity="0.12" />
      {/* cloud body — pixelated silhouette */}
      <path d="M8 18 L8 14 L12 14 L12 10 L18 10 L18 6 L28 6 L28 10 L34 10 L34 14 L40 14 L40 18 L8 18 Z"
            fill={color} />
      <path d="M8 18 L40 18 L40 20 L36 20 L36 22 L12 22 L12 20 L8 20 Z"
            fill={shade} opacity="0.55" />
      {/* highlights */}
      <rect x="14" y="8" width="4" height="2" fill="#fff" opacity="0.7" />
      <rect x="20" y="4" width="6" height="2" fill="#fff" opacity="0.7" />
    </g>
  );
};

const RainDrops = ({ y = 44, intensity = 'light' }) => {
  const drops = intensity === 'heavy'
    ? [{ x: 16, d: 0 }, { x: 22, d: 1 }, { x: 30, d: 2 }, { x: 38, d: 0 }, { x: 44, d: 1 }]
    : [{ x: 20, d: 0 }, { x: 30, d: 1 }, { x: 40, d: 2 }];
  return (
    <g>
      {drops.map((d, i) => (
        <rect key={i}
          x={d.x} y={y + d.d}
          width="2.4" height="6"
          rx="1.2"
          fill="#4FB8FF"
        >
          <animate attributeName="opacity" values="0.3;1;0.3" dur="0.9s" begin={`${i * 0.15}s`} repeatCount="indefinite" />
          <animate attributeName="y" values={`${y + d.d};${y + d.d + 8};${y + d.d}`} dur="0.9s" begin={`${i * 0.15}s`} repeatCount="indefinite" />
        </rect>
      ))}
    </g>
  );
};

const Lightning = ({ x = 30, y = 40 }) => (
  <path d={`M ${x} ${y} L ${x - 4} ${y + 8} L ${x - 1} ${y + 8} L ${x - 3} ${y + 16} L ${x + 5} ${y + 6} L ${x + 1} ${y + 6} L ${x + 3} ${y}`}
        fill="#FFD93D" stroke="#FFB800" strokeWidth="0.6" strokeLinejoin="round" />
);

const SnowFlakes = ({ y = 44 }) => (
  <g fill="#E8F4FF">
    {[18, 28, 38, 46].map((x, i) => (
      <g key={i} transform={`translate(${x} ${y + (i % 2) * 3})`}>
        <rect x="-0.5" y="-3" width="1" height="6" />
        <rect x="-3" y="-0.5" width="6" height="1" />
        <rect x="-2.2" y="-2.2" width="1" height="1" transform="rotate(45)" />
        <animate attributeName="opacity" values="0.4;1;0.4" dur="1.4s" begin={`${i * 0.2}s`} repeatCount="indefinite" />
      </g>
    ))}
  </g>
);

// Icon definitions
const ICONS = {
  clear: (time) => (time === 'night' ? <MoonBody /> : <SunBody />),
  sun: () => <SunBody />,
  moon: () => <MoonBody />,

  partlyCloudy: (time) => (
    <g>
      {time === 'night'
        ? <MoonBody cx={22} cy={22} r={8} />
        : <SunBody cx={22} cy={22} r={8} />}
      <CloudBody cx={38} cy={40} scale={0.85} />
    </g>
  ),

  cloudy: () => <CloudBody cx={32} cy={32} scale={1.1} />,

  overcast: () => (
    <g>
      <CloudBody cx={26} cy={26} scale={0.9} color="#AEB6CC" shade="#6F7895" />
      <CloudBody cx={38} cy={38} scale={1.0} />
    </g>
  ),

  rain: () => (
    <g>
      <CloudBody cx={32} cy={24} scale={1.05} />
      <RainDrops y={38} intensity="light" />
    </g>
  ),

  heavyRain: () => (
    <g>
      <CloudBody cx={32} cy={22} scale={1.1} color="#9AA4BE" shade="#5C6682" />
      <RainDrops y={36} intensity="heavy" />
    </g>
  ),

  storm: () => (
    <g>
      <CloudBody cx={32} cy={22} scale={1.1} color="#7C87A8" shade="#4A5372" />
      <Lightning x={32} y={36} />
      <RainDrops y={42} intensity="light" />
    </g>
  ),

  snow: () => (
    <g>
      <CloudBody cx={32} cy={24} scale={1.05} />
      <SnowFlakes y={40} />
    </g>
  ),

  fog: () => (
    <g>
      <rect x="10" y="22" width="44" height="3" rx="1.5" fill="#E8ECF5" opacity="0.8" />
      <rect x="14" y="30" width="40" height="3" rx="1.5" fill="#E8ECF5" opacity="0.6" />
      <rect x="8" y="38" width="46" height="3" rx="1.5" fill="#E8ECF5" opacity="0.75" />
      <rect x="16" y="46" width="36" height="3" rx="1.5" fill="#E8ECF5" opacity="0.5" />
    </g>
  ),

  wind: () => (
    <g fill="none" stroke="#C7D1E8" strokeWidth="2.5" strokeLinecap="round">
      <path d="M8 22 L40 22 Q48 22 48 16 Q48 10 42 10" />
      <path d="M8 32 L50 32 Q58 32 58 26" />
      <path d="M8 42 L36 42 Q44 42 44 48 Q44 54 38 54" />
    </g>
  ),

  hail: () => (
    <g>
      <CloudBody cx={32} cy={22} scale={1.05} color="#9AA4BE" shade="#5C6682" />
      <circle cx={22} cy={42} r="2.8" fill="#E8F4FF" />
      <circle cx={32} cy={46} r="2.8" fill="#E8F4FF" />
      <circle cx={42} cy={42} r="2.8" fill="#E8F4FF" />
    </g>
  ),
};

Object.assign(window, { WeatherIcon });
