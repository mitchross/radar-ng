// Radar-NG — Alerts list + Alert Detail
// Always-stormy gradient regardless of current weather (this tab is serious).

const SEVERITIES = {
  Extreme:  { color: '#FF3B4A', label: 'EXTREME',  glow: 'rgba(255,59,74,0.35)' },
  Severe:   { color: '#FF8A3A', label: 'SEVERE',   glow: 'rgba(255,138,58,0.30)' },
  Moderate: { color: '#F5D042', label: 'MODERATE', glow: 'rgba(245,208,66,0.25)' },
  Minor:    { color: '#4FB8FF', label: 'MINOR',    glow: 'rgba(79,184,255,0.22)' },
};

function AlertsScreen({ data, onOpenAlert }) {
  const f = window.CUMULUS.fonts;
  const alerts = data.alerts || [];
  const stormBG = 'linear-gradient(180deg, #1a0d2e 0%, #0f1424 50%, #1a0a18 100%)';

  return (
    <PullToRefresh accent="#FF6E7A" onRefresh={() => new Promise(r => setTimeout(r, 600))}>
    <div style={{ width: '100%', minHeight: '100%', background: stormBG, color: '#fff', fontFamily: f.ui, paddingTop: 56, paddingBottom: 100 }}>

      {/* Header */}
      <div style={{ padding: '12px 20px 4px', display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
        <div>
          <div style={{ fontSize: 10, fontFamily: f.mono, color: 'rgba(255,255,255,0.5)', letterSpacing: '0.16em', fontWeight: 700 }}>ACTIVE ALERTS</div>
          <div style={{ fontSize: 28, fontWeight: 700, letterSpacing: -0.5, marginTop: 2 }}>
            {alerts.length === 0 ? 'All clear' : `${alerts.length} in your area`}
          </div>
        </div>
        <div style={{ width: 36, height: 36, borderRadius: 18, background: 'rgba(255,255,255,0.08)', display: 'flex', alignItems: 'center', justifyContent: 'center', cursor: 'pointer' }}>
          <svg width="14" height="14" viewBox="0 0 14 14" fill="none"><path d="M2 7 a5 5 0 0 1 9-3 M12 7 a5 5 0 0 1 -9 3 M11 1 V4 H8 M3 13 V10 H6" stroke="#fff" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round"/></svg>
        </div>
      </div>

      {alerts.length === 0 && (
        <div style={{ padding: '60px 20px', display: 'flex', flexDirection: 'column', alignItems: 'center', textAlign: 'center' }}>
          <div style={{ width: 88, height: 88, borderRadius: 44, background: 'rgba(74,222,128,0.18)', display: 'flex', alignItems: 'center', justifyContent: 'center', boxShadow: '0 0 60px rgba(74,222,128,0.4)' }}>
            <div style={{ width: 16, height: 16, borderRadius: 8, background: '#4ADE80', boxShadow: '0 0 20px #4ADE80' }} />
          </div>
          <div style={{ fontSize: 22, fontWeight: 600, marginTop: 22 }}>No active alerts</div>
          <div style={{ fontSize: 13, color: 'rgba(255,255,255,0.55)', marginTop: 6, maxWidth: 240, lineHeight: 1.5 }}>
            We'll surface NWS warnings, watches, and advisories the moment they're issued.
          </div>
        </div>
      )}

      <div style={{ padding: '14px 14px 0', display: 'flex', flexDirection: 'column', gap: 10 }}>
        {alerts.map((a, i) => <AlertCard key={i} alert={a} onTap={() => onOpenAlert(a)} />)}
      </div>
    </div>
    </PullToRefresh>
  );
}

function AlertCard({ alert, onTap }) {
  const sev = SEVERITIES[alert.severity] || SEVERITIES.Minor;
  const f = window.CUMULUS.fonts;
  return (
    <div onClick={onTap} style={{
      position: 'relative', cursor: 'pointer',
      background: 'rgba(255,255,255,0.06)', borderRadius: 16,
      border: '1px solid rgba(255,255,255,0.08)',
      overflow: 'hidden',
      boxShadow: `0 8px 28px ${sev.glow}`,
    }}>
      <div style={{ position: 'absolute', left: 0, top: 0, bottom: 0, width: 4, background: sev.color }} />
      <div style={{ padding: '14px 16px 14px 18px' }}>
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 8 }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 6, padding: '3px 8px', borderRadius: 6, background: `${sev.color}22`, border: `0.5px solid ${sev.color}55` }}>
            <div style={{ width: 6, height: 6, borderRadius: 3, background: sev.color, boxShadow: `0 0 6px ${sev.color}` }} />
            <div style={{ fontSize: 9, fontFamily: f.mono, fontWeight: 700, color: sev.color, letterSpacing: '0.12em' }}>{sev.label}</div>
          </div>
          <div style={{ fontSize: 10, fontFamily: f.mono, color: 'rgba(255,255,255,0.5)', letterSpacing: '0.06em' }}>{alert.urgency}</div>
        </div>
        <div style={{ fontSize: 17, fontWeight: 700, letterSpacing: -0.2, marginBottom: 4 }}>{alert.event}</div>
        <div style={{ fontSize: 13, color: 'rgba(255,255,255,0.78)', lineHeight: 1.4, marginBottom: 8 }}>{alert.headline}</div>
        <div style={{ fontSize: 11, color: 'rgba(255,255,255,0.5)', display: 'flex', alignItems: 'center', gap: 6 }}>
          <svg width="10" height="10" viewBox="0 0 10 10"><circle cx="5" cy="5" r="4" stroke="rgba(255,255,255,0.4)" strokeWidth="0.8" fill="none"/><path d="M5 2.5 V5 L7 6" stroke="rgba(255,255,255,0.5)" strokeWidth="0.8" fill="none" strokeLinecap="round"/></svg>
          Until {alert.expires}
        </div>
      </div>
    </div>
  );
}

function AlertDetailScreen({ alert, onBack }) {
  const sev = SEVERITIES[alert.severity] || SEVERITIES.Minor;
  const f = window.CUMULUS.fonts;
  const stormBG = 'linear-gradient(180deg, #1a0d2e 0%, #0f1424 60%, #0a0e1a 100%)';
  return (
    <PullToRefresh accent={sev.color} onRefresh={() => new Promise(r => setTimeout(r, 600))}>
    <div style={{ width: '100%', minHeight: '100%', background: stormBG, color: '#fff', fontFamily: f.ui, paddingTop: 50, paddingBottom: 100 }}>

      {/* Severity-tinted hero band */}
      <div style={{ position: 'relative', padding: '14px 16px 20px', background: `linear-gradient(180deg, ${sev.color}55, ${sev.color}11)`, borderBottom: `1px solid ${sev.color}33` }}>
        <div onClick={onBack} style={{ width: 36, height: 36, borderRadius: 18, background: 'rgba(0,0,0,0.3)', display: 'flex', alignItems: 'center', justifyContent: 'center', cursor: 'pointer', marginBottom: 12 }}>
          <svg width="14" height="14" viewBox="0 0 14 14"><path d="M9 2 L4 7 L9 12" stroke="#fff" strokeWidth="1.8" fill="none" strokeLinecap="round" strokeLinejoin="round"/></svg>
        </div>
        <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 10 }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 6, padding: '4px 9px', borderRadius: 6, background: sev.color, color: '#fff' }}>
            <div style={{ width: 6, height: 6, borderRadius: 3, background: '#fff', boxShadow: '0 0 6px #fff' }} />
            <div style={{ fontSize: 10, fontFamily: f.mono, fontWeight: 800, letterSpacing: '0.14em' }}>{sev.label}</div>
          </div>
          <div style={{ fontSize: 11, fontFamily: f.mono, color: 'rgba(255,255,255,0.7)' }}>{alert.urgency}</div>
        </div>
        <div style={{ fontSize: 26, fontWeight: 700, letterSpacing: -0.4, lineHeight: 1.15 }}>{alert.event}</div>
        <div style={{ fontSize: 13, color: 'rgba(255,255,255,0.78)', marginTop: 6 }}>{alert.area}</div>
      </div>

      {/* Polygon mini-map */}
      <div style={{ margin: '14px 16px', borderRadius: 16, overflow: 'hidden', position: 'relative', background: '#0e1726', border: '1px solid rgba(255,255,255,0.08)' }}>
        <svg viewBox="0 0 360 160" style={{ width: '100%', display: 'block' }}>
          <rect width="360" height="160" fill="#0e1726"/>
          {/* Coarse map suggestion */}
          <g stroke="rgba(255,255,255,0.08)" strokeWidth="0.5" fill="none">
            {Array.from({length:8}).map((_,i)=><line key={i} x1={i*45} y1={0} x2={i*45} y2={160}/>)}
            {Array.from({length:6}).map((_,i)=><line key={i} x1={0} y1={i*30} x2={360} y2={i*30}/>)}
          </g>
          {/* County outlines */}
          <g stroke="rgba(255,255,255,0.18)" strokeWidth="0.7" fill="none">
            <path d="M 60 30 L 150 25 L 200 50 L 180 90 L 110 100 L 70 80 Z"/>
            <path d="M 200 50 L 290 40 L 320 90 L 260 120 L 180 90 Z"/>
            <path d="M 110 100 L 180 90 L 260 120 L 240 150 L 130 145 Z"/>
          </g>
          {/* Alert polygon */}
          <path d="M 130 55 L 220 50 L 270 80 L 240 120 L 160 115 Z" fill={sev.color} fillOpacity="0.32" stroke={sev.color} strokeWidth="1.5"/>
          {/* User location */}
          <g>
            <circle cx="200" cy="85" r="14" fill={sev.color} fillOpacity="0.2"/>
            <circle cx="200" cy="85" r="5" fill="#fff" stroke="#0e1726" strokeWidth="2"/>
          </g>
        </svg>
        <div style={{ position: 'absolute', top: 10, left: 12, fontSize: 10, fontFamily: f.mono, color: 'rgba(255,255,255,0.6)', letterSpacing: '0.1em', fontWeight: 700 }}>WARNING POLYGON</div>
        <div style={{ position: 'absolute', bottom: 10, right: 12, padding: '5px 10px', background: 'rgba(255,255,255,0.92)', color: '#0a0e1a', borderRadius: 8, fontSize: 11, fontWeight: 700, cursor: 'pointer' }}>Open in Radar →</div>
      </div>

      {/* Time block */}
      <div style={{ margin: '0 16px 14px', padding: 14, borderRadius: 14, background: 'rgba(255,255,255,0.05)', border: '1px solid rgba(255,255,255,0.08)' }}>
        <div style={{ display: 'grid', gridTemplateColumns: 'repeat(3, 1fr)', gap: 12 }}>
          <Time label="ONSET" value={alert.onset} />
          <Time label="EFFECTIVE" value={alert.effective} />
          <Time label="EXPIRES" value={alert.expires} accent={sev.color} />
        </div>
        <div style={{ marginTop: 12, paddingTop: 10, borderTop: '0.5px solid rgba(255,255,255,0.08)', fontSize: 11, fontFamily: f.mono, color: 'rgba(255,255,255,0.55)' }}>
          Issued by {alert.sender}
        </div>
      </div>

      <Section label="DESCRIPTION">
        <div style={{ fontSize: 14, lineHeight: 1.55, color: 'rgba(255,255,255,0.82)', whiteSpace: 'pre-line' }}>{alert.description}</div>
      </Section>

      <Section label="WHAT TO DO" tint={sev.color}>
        <div style={{ fontSize: 14, lineHeight: 1.55, color: '#fff', whiteSpace: 'pre-line', fontWeight: 500 }}>{alert.instruction}</div>
      </Section>

      <Section label="SOURCE">
        <div style={{ display: 'flex', justifyContent: 'space-between', fontSize: 12, fontFamily: f.mono, color: 'rgba(255,255,255,0.55)' }}>
          <div>{alert.sender}</div>
          <div>NWS CAP · v1.2</div>
        </div>
      </Section>
    </div>
    </PullToRefresh>
  );
}

function Time({ label, value, accent }) {
  const f = window.CUMULUS.fonts;
  return (
    <div>
      <div style={{ fontSize: 9, fontFamily: f.mono, color: 'rgba(255,255,255,0.5)', letterSpacing: '0.12em', fontWeight: 700, marginBottom: 4 }}>{label}</div>
      <div style={{ fontSize: 13, fontFamily: f.mono, fontWeight: 600, color: accent || '#fff' }}>{value}</div>
    </div>
  );
}

function Section({ label, tint, children }) {
  const f = window.CUMULUS.fonts;
  return (
    <div style={{ margin: '0 16px 14px' }}>
      <div style={{ fontSize: 10, fontFamily: f.mono, color: 'rgba(255,255,255,0.5)', letterSpacing: '0.16em', fontWeight: 700, padding: '6px 4px 8px' }}>{label}</div>
      <div style={{ padding: 14, borderRadius: 14, background: tint ? `${tint}15` : 'rgba(255,255,255,0.05)', border: `1px solid ${tint ? tint + '33' : 'rgba(255,255,255,0.08)'}` }}>
        {children}
      </div>
    </div>
  );
}

Object.assign(window, { AlertsScreen, AlertDetailScreen });
