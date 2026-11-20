# Gaia DR3 明亮恒星样本

数据来源为 ESA 的 Gaia 任务，由 Gaia Data Processing and Analysis Consortium 处理。数据署名为 **ESA/Gaia/DPAC**。[Gaia DR3 引用说明](https://gea.esac.esa.int/archive/documentation/GDR3/Miscellaneous/sec_credit_and_citation_instructions/) · [Gaia DR3 原始表定义](https://gea.esac.esa.int/archive/documentation/GDR3/Gaia_archive/chap_datamodel/sec_dm_main_source_catalogue/ssec_dm_gaia_source.html)

该文件为真实观测子集。它选取满足视差和信噪比条件的最亮 8,000 个条目，距离采用观测视差的倒数，由毫角秒换算为千光年。颜色依据蓝红光度差近似。该样本主要位于太阳附近，并且偏向明亮、视差测量较可靠的对象。星盘、旋臂和远处背景仍为示意。

```sql
SELECT TOP 8000 source_id, l, b, parallax, parallax_over_error, phot_g_mean_mag, bp_rp
FROM gaiadr3.gaia_source
WHERE parallax > 0.5 AND parallax_over_error >= 25
  AND phot_g_mean_mag < 12 AND bp_rp IS NOT NULL
ORDER BY phot_g_mean_mag, source_id
```

查询服务为 [Gaia ESA Archive TAP](https://gea.esac.esa.int/tap-server/tap)。输出列包含 Gaia DR3 源编号、银经、银纬、距离（千光年）、G 星等及 BP–RP 色指数。
