/** Detail sheets for tapped storm cells and warning polygons on the radar map. */
import { Text } from "react-native";
import { MapInfoSheet, PlainEnglish } from "./MapInfoSheet";
import { warningColor, type WarningDetails } from "./WarningsOverlay";
import { compass, hailLabel, isTracked, type StormCellInfo } from "../../lib/stormTracks";
import { useAlertExplanation } from "../../hooks/useAlertExplanation";
import { cumulus } from "../../lib/cumulusTheme";

const dbzColor = (dbz: number) => (dbz >= 60 ? "#d02058" : dbz >= 55 ? "#ff4040" : dbz >= 50 ? "#ff8a00" : "#e0a800");

export function StormCellSheet({ cell, onClose }: { cell: StormCellInfo | null; onClose: () => void }) {
  if (!cell) return null;
  const tracked = isTracked(cell);
  return (
    <MapInfoSheet
      visible
      eyebrow="STORM CELL"
      title={`${Math.round(cell.peakDbz)} dBZ peak`}
      subtitle={tracked ? `Moving ${compass(cell.bearing)} at ${Math.round(cell.speedMph)} mph` : "Motion not yet tracked"}
      accent={dbzColor(cell.peakDbz)}
      pill={hailLabel(cell.peakDbz)}
      metrics={[
        { label: "AREA", value: Math.round(cell.areaKm2).toLocaleString(), unit: "km²" },
        { label: "SPEED", value: tracked ? `${Math.round(cell.speedMph)}` : "—", unit: "mph" },
      ]}
      onClose={onClose}
    />
  );
}

export function WarningSheet({ warning, onClose }: { warning: WarningDetails | null; onClose: () => void }) {
  const { explanation, loading } = useAlertExplanation(warning?.id);
  if (!warning) return null;
  const until = warning.ends ? new Date(warning.ends).toLocaleString([], { weekday: "short", hour: "numeric", minute: "2-digit" }) : null;
  return (
    <MapInfoSheet
      visible
      eyebrow={warning.event.toUpperCase()}
      title={(warning.areaDesc ?? "").split(";").slice(0, 3).join(", ") || warning.event}
      subtitle={until ? `Until ${until}` : warning.severity}
      accent={warningColor(warning.event)}
      onClose={onClose}
    >
      {explanation ? <PlainEnglish what={explanation.what} doText={explanation.do} /> : null}
      {!explanation && loading ? <Text style={{ marginTop: 12, color: cumulus.inkMuted, fontSize: 12 }}>{"✦"} Explaining…</Text> : null}
    </MapInfoSheet>
  );
}
