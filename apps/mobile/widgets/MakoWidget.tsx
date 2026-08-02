/** Deprecated widget-kind bridge for widgets installed before the Hive rename. */
import { createWidget } from "expo-widgets";
import { HiveWidgetView } from "./HiveWidget";

export default createWidget("MakoWidget", HiveWidgetView);
