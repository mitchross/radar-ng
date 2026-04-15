import { useState } from "react";
import {
  View,
  Text,
  StyleSheet,
  ScrollView,
  Pressable,
  Modal,
  SafeAreaView,
  useWindowDimensions,
} from "react-native";
import { useForecast } from "../../hooks/useForecast";
import { CurrentConditions } from "./CurrentConditions";
import { HourlyScroll } from "./HourlyScroll";
import { DailyForecast } from "./DailyForecast";
import { getWeatherInfo } from "../../lib/weatherCodes";

export function ForecastPeek() {
  const { data: forecast } = useForecast();
  const [showModal, setShowModal] = useState(false);

  if (!forecast) return null;

  const weather = getWeatherInfo(forecast.current.weather_code);
  const high = Math.round(forecast.daily.temperature_2m_max[0]);
  const low = Math.round(forecast.daily.temperature_2m_min[0]);

  return (
    <>
      <Pressable onPress={() => setShowModal(true)} style={styles.peek}>
        <Text style={styles.peekTemp}>
          {Math.round(forecast.current.temperature_2m)}{"\u00B0"}
        </Text>
        <Text style={styles.peekCondition}>
          {weather.icon} {weather.label}
        </Text>
        <Text style={styles.peekHighLow}>
          {high}{"\u00B0"}/{low}{"\u00B0"}
        </Text>
        <Text style={styles.peekChevron}>{"\u25B2"}</Text>
      </Pressable>

      <Modal
        visible={showModal}
        animationType="slide"
        transparent
        onRequestClose={() => setShowModal(false)}
      >
        <Pressable style={styles.modalBackdrop} onPress={() => setShowModal(false)}>
          <View />
        </Pressable>
        <View style={styles.modalSheet}>
          <Pressable onPress={() => setShowModal(false)} style={styles.modalHandle}>
            <View style={styles.handleBar} />
          </Pressable>
          <ScrollView showsVerticalScrollIndicator={false}>
            <CurrentConditions forecast={forecast} />
            <HourlyScroll forecast={forecast} />
            <DailyForecast forecast={forecast} />
            <View style={{ height: 40 }} />
          </ScrollView>
        </View>
      </Modal>
    </>
  );
}

// Keep the old name as re-export for backward compat
export const ForecastSheet = ForecastPeek;

const styles = StyleSheet.create({
  peek: {
    flexDirection: "row",
    alignItems: "center",
    backgroundColor: "#1a1a2e",
    paddingHorizontal: 16,
    paddingVertical: 10,
    gap: 8,
  },
  peekTemp: {
    color: "#fff",
    fontSize: 18,
    fontWeight: "700",
  },
  peekCondition: {
    color: "#bbb",
    fontSize: 14,
    flex: 1,
  },
  peekHighLow: {
    color: "#888",
    fontSize: 13,
    fontVariant: ["tabular-nums"],
  },
  peekChevron: {
    color: "#555",
    fontSize: 10,
  },
  modalBackdrop: {
    flex: 1,
    backgroundColor: "rgba(0,0,0,0.4)",
  },
  modalSheet: {
    backgroundColor: "#1a1a2e",
    borderTopLeftRadius: 20,
    borderTopRightRadius: 20,
    maxHeight: "80%",
    paddingBottom: 20,
  },
  modalHandle: {
    alignItems: "center",
    paddingVertical: 12,
  },
  handleBar: {
    width: 40,
    height: 4,
    borderRadius: 2,
    backgroundColor: "#555",
  },
});
