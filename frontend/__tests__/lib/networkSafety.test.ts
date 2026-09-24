import { isCleartextToPublicHost } from "../../src/lib/networkSafety";

describe("isCleartextToPublicHost", () => {
  it("allows https anywhere", () => {
    expect(isCleartextToPublicHost("https://radar-ng-api.vanillax.me")).toBe(false);
  });

  it("allows http to hosts on the user's own network", () => {
    for (const url of [
      "http://192.168.1.20:8000",
      "http://10.0.0.5",
      "http://172.20.1.1",
      "http://127.0.0.1:8000",
      "http://localhost:8000",
      "http://radar.local",
      "http://nas.home.arpa",
      "http://radar",
      "http://100.101.1.2",
      "http://[fd12:3456::1]:8000",
      "http://[::1]",
    ]) {
      expect(isCleartextToPublicHost(url)).toBe(false);
    }
  });

  it("flags http to public hosts and addresses", () => {
    for (const url of ["http://radar-ng-api.vanillax.me", "http://8.8.8.8", "http://172.32.0.1", "HTTP://Example.com/path"]) {
      expect(isCleartextToPublicHost(url)).toBe(true);
    }
  });
});
