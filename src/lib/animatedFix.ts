// React Native 0.83's AnimatedNode.__callListeners uses Map#forEach, but
// AnimatedValueXY (and a couple of third-party Animated subclasses, including
// the ones @maplibre/maplibre-react-native builds via
// `Object.getPrototypeOf(Animated.ValueXY)`) overwrite `_listeners` with a
// plain `{}`. When the native animated module reports an update for one of
// those nodes the inherited __callListeners throws:
//
//   TypeError: this._listeners.forEach is not a function (it is undefined)
//
// The throw is harmless (those subclasses don't actually need __callListeners
// to fire — they fan updates out via their own x/y children), but it pollutes
// the dev red-screen and bloats the OTEL error log. Patch the prototype so
// the method no-ops when `_listeners` doesn't have a Map-shaped forEach.
import { Animated } from "react-native";

type AnimatedNodeProto = {
  __callListeners?: (value: number) => void;
  _listeners?: unknown;
};

// Inheritance: AnimatedValue → AnimatedWithChildren → AnimatedNode. Two
// prototype hops from `Animated.Value.prototype` lands on AnimatedNode's
// prototype, which is the one that owns the throwing __callListeners.
// AnimatedWithChildren.__callListeners reaches it via `super`, so patching
// the AnimatedNode level catches every code path.
const withChildrenProto = Object.getPrototypeOf(
  Animated.Value.prototype,
) as object;
const animatedNodeProto = Object.getPrototypeOf(
  withChildrenProto,
) as AnimatedNodeProto;

if (
  animatedNodeProto &&
  typeof animatedNodeProto.__callListeners === "function"
) {
  const original = animatedNodeProto.__callListeners;
  animatedNodeProto.__callListeners = function patched(
    this: AnimatedNodeProto,
    value: number,
  ) {
    const listeners = this._listeners as { forEach?: unknown } | undefined;
    if (listeners && typeof listeners.forEach === "function") {
      original.call(this, value);
    }
  };
}
