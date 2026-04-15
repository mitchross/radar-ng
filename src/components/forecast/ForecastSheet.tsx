import { useState } from "react";
import {
  View,
  Text,
  StyleSheet,
  ScrollView,
  Pressable,
  Modal,
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
        <Text style={styles.peekDivider}>{"\u2022"}</Text>
        <Text style={styles.peekCondition}>{weather.label}</Text>
        <View style={styles.peekSpacer} />
        <Text style={styles.peekHighLow}>
          {high}{"\u00B0"}/{low}{"\u00B0"}
        </Text>
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

export const ForecastSheet = ForecastPeek;

const styles = StyleSheet.create({
  peek: {
    flexDirection: "row",
    alignItems: "center",
    backgroundColor: "rgba(0, 0, 0, 0.7)",
    paddingHorizontal: 14,
    paddingVertical: 6,
    gap: 5,
  },
  peekTemp: {
    color: "#fff",
    fontSize: 13,
    fontWeight: "700",
  },
  peekDivider: {
    color: "#555",
    fontSize: 10,
  },
  peekCondition: {
    color: "#999",
    fontSize: 12,
  },
  peekSpacer: {
    flex: 1,
  },
  peekHighLow: {
    color: "#666",
    fontSize: 12,
    fontVariant: ["tabular-nums"],
  },
  modalBackdrop: {
    flex: 1,
    backgroundColor: "rgba(0,0,0,0.5)",
  },
  modalSheet: {
    backgroundColor: "#0a0a14",
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
    backgroundColor: "#444",
  },
});
