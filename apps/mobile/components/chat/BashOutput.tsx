import { View, Text, ScrollView, StyleSheet } from 'react-native';
import { useThemeContext } from '../../hooks/useTheme';

interface BashOutputProps {
  command?: string;
  output: string;
}

export function BashOutput({ command, output }: BashOutputProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;

  return (
    <View style={[styles.terminal, { borderColor: t.border }]}>
      {/* macOS window dots */}
      <View style={styles.titleBar}>
        <View style={[styles.dot, { backgroundColor: '#ff5f57' }]} />
        <View style={[styles.dot, { backgroundColor: '#febc2e' }]} />
        <View style={[styles.dot, { backgroundColor: '#28c840' }]} />
      </View>

      <ScrollView style={styles.body} horizontal={false} showsVerticalScrollIndicator={false}>
        {command && (
          <Text style={styles.commandLine} selectable>
            <Text style={styles.prompt}>$ </Text>
            <Text style={styles.command}>{command}</Text>
          </Text>
        )}
        {output ? (
          <Text style={[styles.output, { color: t.foreground }]} selectable>
            {output}
          </Text>
        ) : null}
      </ScrollView>
    </View>
  );
}

const styles = StyleSheet.create({
  terminal: {
    backgroundColor: 'rgba(0,0,0,0.4)',
    borderRadius: 10,
    overflow: 'hidden',
    borderWidth: StyleSheet.hairlineWidth,
    marginVertical: 4,
  },
  titleBar: {
    flexDirection: 'row',
    gap: 6,
    paddingHorizontal: 10,
    paddingVertical: 8,
    borderBottomWidth: StyleSheet.hairlineWidth,
    borderBottomColor: 'rgba(255,255,255,0.06)',
  },
  dot: {
    width: 10,
    height: 10,
    borderRadius: 5,
  },
  body: {
    padding: 12,
    maxHeight: 250,
  },
  commandLine: {
    fontFamily: 'Courier',
    fontSize: 13,
    lineHeight: 18,
    marginBottom: 4,
  },
  prompt: {
    color: '#22c55e',
    fontFamily: 'Courier',
  },
  command: {
    color: '#fff',
    fontFamily: 'Courier',
  },
  output: {
    fontFamily: 'Courier',
    fontSize: 13,
    lineHeight: 18,
  },
});
