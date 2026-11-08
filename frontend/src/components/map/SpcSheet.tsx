import { ScrollView, Text, View, useWindowDimensions } from "react-native";
import { MapInfoSheet } from "./MapInfoSheet";
import { PointOutlooks } from "./PointOutlooks";
import { spcStyle } from "../../lib/spc";
import type { SpcSelection } from "./SpcOverlay";

export function SpcSheet({ selection, onClose }: { selection: SpcSelection | null; onClose: () => void }) {
  const { height } = useWindowDimensions();
  if (!selection) return null;
  const { product, lat, lon } = selection;
  const style = spcStyle(product.type);
  return <MapInfoSheet visible eyebrow="NOAA/NWS Storm Prediction Center" title={`${style.name} #${product.number}`}
    subtitle={`Issued ${new Date(product.issued).toLocaleString()}\nUntil ${new Date(product.expires).toLocaleString()}`}
    accent={style.color} onClose={onClose}>
    <ScrollView style={{ maxHeight: height * 0.5 }}>
      <Text style={{ color: "#4c5770", marginVertical: 8 }}>{product.geometry_note}</Text>
      <Text selectable style={{ color: "#1a2030", fontSize: 14, lineHeight: 20 }}>{product.text}</Text>
      {product.probabilities.length > 0 ? <View accessibilityLabel="Watch probabilities" style={{ marginTop: 12 }}>
        <Text style={{ color: "#1a2030", fontWeight: "700" }}>Watch probabilities</Text>
        {product.probabilities.map((row) => <View key={row.label} style={{ flexDirection: "row", gap: 8, paddingVertical: 5 }}>
          <Text style={{ flex: 1, color: "#1a2030", fontSize: 12 }}>{row.label}</Text>
          <Text style={{ color: "#1a2030", fontWeight: "700" }}>{row.value}</Text>
        </View>)}
      </View> : null}
      <PointOutlooks lat={lat} lon={lon} />
    </ScrollView>
  </MapInfoSheet>;
}
