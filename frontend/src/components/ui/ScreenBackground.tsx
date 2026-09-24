import { LinearGradient } from "expo-linear-gradient";
import type { ComponentProps } from "react";
import { View } from "react-native";

type Props = Omit<ComponentProps<typeof LinearGradient>, "colors"> & {
  colors: readonly [string, string, ...string[]];
};

/**
 * A screen's backdrop. When every stop is the same colour a gradient only
 * costs a native layer, so render a plain View; real gradients still work.
 */
export function ScreenBackground({ colors, style, ...rest }: Props) {
  if (colors.every((c) => c === colors[0])) {
    return <View {...rest} style={[style, { backgroundColor: colors[0] }]} />;
  }
  return <LinearGradient {...rest} colors={colors} style={style} />;
}
