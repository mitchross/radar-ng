import { useState } from "react";
import {
  View,
  Text,
  StyleSheet,
  ScrollView,
  Pressable,
  useWindowDimensions,
} from "react-native";
import { useForecast } from "../../hooks/useForecast";
import { CurrentConditions } from "./CurrentConditions";
import { HourlyScroll } from "./HourlyScroll";
import { DailyForecast } from "./DailyForecast";
import { getWeatherInfo } from "../../lib/weatherCodes";

type SheetState = "collapsed" | "half" | "full";

export function ForecastSheet() {
  const { data: forecast, isLoading } = useForecast();
  const [state, setState] = useState<SheetState>("collapsed");
  const { height: screenHeight } = useWindowDimensions();

  const heights: Record<SheetState, number> = {
    collapsed: 56,
    half: screenHeight * 0.4,
    full: screenHeight * 0.75,
  };

  const cycleState = () => {
    const order: SheetState[] = ["collapsed", "half", "full"];
    const idx = order.indexOf(state);
    setState(order[(idx + 1) % order.length]);
  };

  if (!forecast && !isLoading) return null;

  const weather = forecast ? getWeatherInfo(forecast.current.weather_code) : null;

  return (
    <View style={[styles.container, { height: heights[state] }]}>
      <Pressable onPress={cycleState} style={styles.handleTouchArea}>
        <View style={styles.handleBar} />
        {state === "collapsed" && forecast && (
          <View style={styles.peekRow}>
            <Text style={styles.peekTemp}>
              {Math.round(forecast.current.temperature_2m)}{"\u00B0"}
            </Text>
            <Text style={styles.peekCondition}>
              {weather?.icon} {weather?.label}
            </Text>
            <Text style={styles.peekChevron}>{"\u25B2"}</Text>
          </View>
        )}
      </Pressable>
      {state !== "collapsed" && (
        isLoading ? (
          <View style={styles.loading}>
            <Text style={styles.loadingText}>Loading forecast...</Text>
          </View>
        ) : forecast ? (
          <ScrollView
            scrollEnabled={state === "full"}
            showsVerticalScrollIndicator={false}
          >
            <CurrentConditions forecast={forecast} />
            <HourlyScroll forecast={forecast} />
            {state === "full" && <DailyForecast forecast={forecast} />}
          </ScrollView>
        ) : null
      )}
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    backgroundColor: "#1a1a2e",
    borderTopLeftRadius: 16,
    borderTopRightRadius: 16,
    overflow: "hidden",
  },
  handleTouchArea: {
    alignItems: "center",
    paddingTop: 8,
    paddingBottom: 6,
    minHeight: 44,
  },
  handleBar: {
    width: 36,
    height: 4,
    borderRadius: 2,
    backgroundColor: "#555",
    marginBottom: 4,
  },
  peekRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    paddingHorizontal: 16,
  },
  peekTemp: {
    color: "#fff",
    fontSize: 18,
    fontWeight: "600",
  },
  peekCondition: {
    color: "#aaa",
    fontSize: 14,
    flex: 1,
  },
  peekChevron: {
    color: "#555",
    fontSize: 10,
  },
  loading: {
    padding: 20,
    alignItems: "center",
  },
  loadingText: {
    color: "#888",
    fontSize: 14,
  },
});
