// Radar-NG — Pull to refresh wrapper
// Wraps a scrollable container; reveals an iOS-style spinner+arrow when user pulls past threshold.

function PullToRefresh({ children, onRefresh, accent }) {
  const [pull, setPull] = React.useState(0);     // 0..120
  const [refreshing, setRefreshing] = React.useState(false);
  const [hint, setHint] = React.useState('Pull to refresh');
  const startY = React.useRef(null);
  const scrollRef = React.useRef(null);
  const accentColor = accent || '#8B7CFF';

  const THRESHOLD = 64;
  const MAX_PULL = 110;

  const onTouchStart = (e) => {
    if (refreshing) return;
    if (scrollRef.current && scrollRef.current.scrollTop > 0) return;
    startY.current = e.touches ? e.touches[0].clientY : e.clientY;
  };
  const onTouchMove = (e) => {
    if (startY.current === null || refreshing) return;
    if (scrollRef.current && scrollRef.current.scrollTop > 0) { startY.current = null; setPull(0); return; }
    const cy = e.touches ? e.touches[0].clientY : e.clientY;
    const delta = cy - startY.current;
    if (delta > 0) {
      e.preventDefault?.();
      const eased = Math.min(MAX_PULL, delta * 0.55);
      setPull(eased);
      setHint(eased > THRESHOLD ? 'Release to refresh' : 'Pull to refresh');
    }
  };
  const onTouchEnd = async () => {
    if (startY.current === null) return;
    const wasPastThreshold = pull > THRESHOLD;
    startY.current = null;
    if (wasPastThreshold) {
      setRefreshing(true);
      setPull(48); // hold at "refreshing" position
      setHint('Updating · MRMS · HRRR · NWS');
      try { await (onRefresh && onRefresh()); } catch(e) {}
      // mock minimum spinner duration so it feels real
      await new Promise(r => setTimeout(r, 900));
      setHint('Updated just now');
      setTimeout(() => { setRefreshing(false); setPull(0); }, 400);
    } else {
      setPull(0);
    }
  };

  const progress = Math.min(1, pull / THRESHOLD);
  const f = window.CUMULUS.fonts;

  return (
    <div style={{ width: '100%', height: '100%', position: 'relative', overflow: 'hidden' }}
         onTouchStart={onTouchStart} onTouchMove={onTouchMove} onTouchEnd={onTouchEnd}
         onMouseDown={onTouchStart} onMouseMove={(e) => startY.current !== null && onTouchMove(e)} onMouseUp={onTouchEnd} onMouseLeave={() => startY.current && onTouchEnd()}>

      {/* Indicator pinned to top */}
      <div style={{
        position: 'absolute', top: 0, left: 0, right: 0,
        height: pull, zIndex: 5,
        display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'flex-end',
        paddingBottom: 6,
        pointerEvents: 'none',
        opacity: pull > 6 ? 1 : 0,
        transition: refreshing || pull === 0 ? 'height 0.28s cubic-bezier(.2,.7,.3,1), opacity 0.2s' : 'none',
      }}>
        <Spinner active={refreshing} progress={progress} accent={accentColor} />
        <div style={{ fontSize: 10, fontFamily: f.mono, color: 'rgba(255,255,255,0.6)', letterSpacing: '0.1em', marginTop: 4, fontWeight: 600 }}>
          {hint}
        </div>
      </div>

      {/* Scrollable content — translates down with pull */}
      <div ref={scrollRef} style={{
        width: '100%', height: '100%', overflow: 'auto',
        transform: `translateY(${pull}px)`,
        transition: refreshing || pull === 0 ? 'transform 0.28s cubic-bezier(.2,.7,.3,1)' : 'none',
        WebkitOverflowScrolling: 'touch',
      }}>
        {children}
      </div>
    </div>
  );
}

function Spinner({ active, progress, accent }) {
  if (active) {
    return (
      <div style={{ width: 22, height: 22, position: 'relative' }}>
        <style>{`@keyframes ptrSpin { to { transform: rotate(360deg); } }`}</style>
        <svg width="22" height="22" viewBox="0 0 22 22" style={{ animation: 'ptrSpin 0.9s linear infinite' }}>
          <circle cx="11" cy="11" r="8" stroke="rgba(255,255,255,0.18)" strokeWidth="2" fill="none"/>
          <path d="M 11 3 A 8 8 0 0 1 19 11" stroke={accent} strokeWidth="2" fill="none" strokeLinecap="round"/>
        </svg>
      </div>
    );
  }
  // Arc that fills as you pull
  const r = 8;
  const c = 2 * Math.PI * r;
  return (
    <div style={{ width: 22, height: 22 }}>
      <svg width="22" height="22" viewBox="0 0 22 22" style={{ transform: `rotate(${progress * 180}deg)`, transition: 'transform 0.05s' }}>
        <circle cx="11" cy="11" r={r} stroke="rgba(255,255,255,0.18)" strokeWidth="2" fill="none"/>
        <circle cx="11" cy="11" r={r} stroke={accent} strokeWidth="2" fill="none" strokeLinecap="round"
                strokeDasharray={c} strokeDashoffset={c * (1 - progress)}
                transform="rotate(-90 11 11)"/>
      </svg>
    </div>
  );
}

Object.assign(window, { PullToRefresh });
