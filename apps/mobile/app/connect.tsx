import { useEffect } from "react";
import { View } from "react-native";
import { useRouter } from "expo-router";

/**
 * Landing screen for `mitsuro://connect?url=...&token=...`.
 *
 * `useDeepLink` in the root layout applies the credentials. This route exists
 * so Expo Router does not treat the host as an unmatched page.
 */
export default function ConnectScreen() {
  const router = useRouter();

  useEffect(() => {
    router.replace("/");
  }, [router]);

  return <View style={{ flex: 1 }} />;
}
