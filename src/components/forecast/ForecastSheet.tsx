import { useState } from "react";
import {
  View,
  Text,
  StyleSheet,
  ScrollView,
  TouchableOpacity,
  Dimensions,
  useWindowDimensions,
} from "react-native";
import { useForecast } from "../../hooks/useForecast";
import { CurrentConditions } from "./CurrentConditions";
import { HourlyScroll } from "./HourlyScroll";
import { DailyForecast } from "./DailyForecast";

type SheetState = "collapsed" | "half" | "full";

export function ForecastSheet() {
  const { data: forecast, isLoading } = useForecast();
  const [state, setState] = useState<SheetState>("collapsed");
  const { height: screenHeight } = useWindowDimensions();

  const heights: Record<SheetState, number> = {
    collapsed: 80,
    half: screenHeight * 0.35,
    full: screenHeight * 0.8,
  };

  const cycleState = () => {
    const order: SheetState[] = ["collapsed", "half", "full"];
    const idx = order.indexOf(state);
    setState(order[(idx + 1) % order.length]);
  };

  if (!forecast && !isLoading) return null;

  return (
    <View style={[styles.container, { height: heights[state] }]}>
      <TouchableOpacity onPress={cycleState} activeOpacity={0.9}>
        <View style={styles.handle}>
          <View style={styles.handleBar} />
        </View>
      </TouchableOpacity>
      {isLoading ? (
        <View style={styles.loading}>
          <Text style={styles.loadingText}>Loading forecast...</Text>
        </View>
      ) : forecast ? (
        <ScrollView
          scrollEnabled={state !== "collapsed"}
          showsVerticalScrollIndicator={false}
        >
          <CurrentConditions forecast={forecast} />
          {state !== "collapsed" && <HourlyScroll forecast={forecast} />}
          {state === "full" && <DailyForecast forecast={forecast} />}
        </ScrollView>
      ) : null}
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
  handle: {
    alignItems: "center",
    paddingTop: 10,
    paddingBottom: 4,
  },
  handleBar: {
    width: 40,
    height: 4,
    borderRadius: 2,
    backgroundColor: "#555",
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
