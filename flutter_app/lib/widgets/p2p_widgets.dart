import 'package:material_ui/material_ui.dart';

import '../app_theme.dart';

class PageHeading extends StatelessWidget {
  const PageHeading({
    super.key,
    required this.eyebrow,
    required this.title,
    required this.subtitle,
    this.trailing,
  });

  final String eyebrow;
  final String title;
  final String subtitle;
  final Widget? trailing;

  @override
  Widget build(BuildContext context) {
    return Row(
      crossAxisAlignment: CrossAxisAlignment.center,
      children: [
        Expanded(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(
                eyebrow.toUpperCase(),
                style: const TextStyle(
                  color: P2PColors.mint,
                  fontSize: 11,
                  fontWeight: FontWeight.w800,
                  letterSpacing: 1.5,
                ),
              ),
              const SizedBox(height: 9),
              Text(
                title,
                style: Theme.of(context).textTheme.headlineMedium?.copyWith(
                      color: P2PColors.text,
                      fontWeight: FontWeight.w800,
                      letterSpacing: -0.7,
                    ),
              ),
              const SizedBox(height: 5),
              Text(
                subtitle,
                style: const TextStyle(color: P2PColors.muted, fontSize: 13),
              ),
            ],
          ),
        ),
        if (trailing != null) ...[
          const SizedBox(width: 16),
          trailing!,
        ],
      ],
    );
  }
}

class PrototypePill extends StatelessWidget {
  const PrototypePill({super.key});

  @override
  Widget build(BuildContext context) {
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
      decoration: BoxDecoration(
        color: const Color(0xFF3D3020),
        border: Border.all(color: const Color(0xFF755A31)),
        borderRadius: BorderRadius.circular(30),
      ),
      child: const Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          Icon(Icons.science_outlined, size: 14, color: P2PColors.amber),
          SizedBox(width: 7),
          Text(
            'UI 原型 · 演示数据',
            style: TextStyle(
              color: P2PColors.amber,
              fontSize: 11,
              fontWeight: FontWeight.w700,
            ),
          ),
        ],
      ),
    );
  }
}

class PanelCard extends StatelessWidget {
  const PanelCard({
    super.key,
    required this.child,
    this.padding = const EdgeInsets.all(22),
    this.gradient,
  });

  final Widget child;
  final EdgeInsetsGeometry padding;
  final Gradient? gradient;

  @override
  Widget build(BuildContext context) {
    return Container(
      padding: padding,
      decoration: BoxDecoration(
        gradient: gradient,
        color: gradient == null ? P2PColors.panel : null,
        borderRadius: BorderRadius.circular(20),
        border: Border.all(color: P2PColors.outline),
        boxShadow: const [
          BoxShadow(
            color: Color(0x22000000),
            blurRadius: 22,
            offset: Offset(0, 9),
          ),
        ],
      ),
      child: Material(
        color: Colors.transparent,
        borderRadius: BorderRadius.circular(20),
        clipBehavior: Clip.antiAlias,
        child: child,
      ),
    );
  }
}

class SectionTitle extends StatelessWidget {
  const SectionTitle({
    super.key,
    required this.title,
    this.actionLabel,
    this.onAction,
  });

  final String title;
  final String? actionLabel;
  final VoidCallback? onAction;

  @override
  Widget build(BuildContext context) {
    return Row(
      children: [
        Expanded(
          child: Text(
            title,
            style: Theme.of(context).textTheme.titleLarge?.copyWith(
                  fontWeight: FontWeight.w800,
                  color: P2PColors.text,
                ),
          ),
        ),
        if (actionLabel != null)
          TextButton(onPressed: onAction, child: Text(actionLabel!)),
      ],
    );
  }
}

class StatusDot extends StatelessWidget {
  const StatusDot({super.key, required this.color, this.label});

  final Color color;
  final String? label;

  @override
  Widget build(BuildContext context) {
    return Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        Container(
          width: 7,
          height: 7,
          decoration: BoxDecoration(color: color, shape: BoxShape.circle),
        ),
        if (label != null) ...[
          const SizedBox(width: 7),
          Text(label!, style: TextStyle(color: color, fontSize: 12)),
        ],
      ],
    );
  }
}

class DeviceTile extends StatelessWidget {
  const DeviceTile({
    super.key,
    required this.name,
    required this.detail,
    required this.online,
    required this.icon,
    this.onConnect,
  });

  final String name;
  final String detail;
  final bool online;
  final IconData icon;
  final VoidCallback? onConnect;

  @override
  Widget build(BuildContext context) {
    return PanelCard(
      padding: const EdgeInsets.all(17),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Row(
            children: [
              Container(
                width: 43,
                height: 43,
                decoration: BoxDecoration(
                  color: P2PColors.panelRaised,
                  borderRadius: BorderRadius.circular(13),
                ),
                child: Icon(icon, color: P2PColors.mint, size: 22),
              ),
              const Spacer(),
              StatusDot(
                color: online ? P2PColors.mint : P2PColors.muted,
                label: online ? '在线' : '离线',
              ),
            ],
          ),
          const SizedBox(height: 16),
          Text(name, style: const TextStyle(fontWeight: FontWeight.w800, fontSize: 16)),
          const SizedBox(height: 5),
          Text(detail, style: const TextStyle(color: P2PColors.muted, fontSize: 12)),
          const SizedBox(height: 15),
          SizedBox(
            width: double.infinity,
            child: FilledButton.tonal(
              onPressed: onConnect,
              style: FilledButton.styleFrom(
                backgroundColor: P2PColors.panelRaised,
                foregroundColor: P2PColors.mint,
                minimumSize: const Size.fromHeight(40),
              ),
              child: const Text('连接预览'),
            ),
          ),
        ],
      ),
    );
  }
}
